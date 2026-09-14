# Real-time Task Status via SSE — Design

**Date:** 2026-09-14 (updated from 2026-09-09)
**Status:** Proposed
**Scope:** Relay accurate, up-to-date, low-latency **background-task status** to HTMX clients. The `task_status` table is a **first-class citizen** — focused purely on the task-state problem, which is non-trivial on its own. Signaling/tracking **capture lifecycle events** (created/deleted elsewhere) is explicitly **TBD** and out of scope for this phase.

---

## 1. Problem statement

Today, when a user uploads a screenshot, the app enqueues an AI-powered illumination in the background. There is **no notification to the client** when it completes — the user must guess and manually refresh the page to see the updated illumination data.

We want a **simple, idiomatic, robust, and flexible** strategy for relaying accurate, up-to-date, low-latency background-task status to clients. It must:

- Scale across different **task types** (illumination, spark, search-index, ingest).
- Let the client **subscribe to only what it cares about** (avoid noise, e.g. during a backfill).
- Handle **reruns** (e.g. "illuminate this again with a new model").
- **Respect the connection budget** of our narrow topology — long-lived SSE connections must not be held open indefinitely when idle.
- Minimize frontend cruft/complexity (the author is not a JS coder).

> **Scope boundary:** this phase is about **task status only**. The `task_status` table stays focused on task state. Capture lifecycle events (a capture uploaded/deleted on another device) are a **separate, TBD concern** — see §13. We deliberately do **not** generalize `task_status` into a catch-all event table.

---

## 2. Current architecture (as reviewed)

### 2.1 The upload → illumination flow

```
Upload (webui/v2/r_upload.rs)
  └─ insert_capture() → beacon.signal_new_capture()   [IngestTask]
       └─ /_wh/cloudtask/ingest → logic/ingest
            └─ beacon.signal_illumination()            [IlluminationTask]
                 └─ /_wh/cloudtask/illuminate → logic/illuminate::exec
                      └─ insert_illumination()  ← row written, nobody told
```

### 2.2 Key facts that shape the design

- **`TaskQueue::get_status()` is `unimplemented!()`** in all three backends (`taskqueue_local.rs`, `taskqueue_pubsub.rs`, `taskqueue_cloudtask.rs`). The trait already anticipates status queries — it's just never been filled in.
- **`OneShotQueue` is dead code and will be removed.** It was an old local-only emulation of task execution and is **not used anywhere in production** — it appears only in `src/common/mod.rs` (module decl + re-export) and its own file `src/common/oneshotqueue.rs` (definition + unit tests). The `LocalTaskQueue` (in-process mpsc + semaphore) is the real local backend and does **not** dedupe. So the rerun problem is *not* caused by `OneShotQueue`; it's caused by the **idempotency guard in `logic/illuminate.rs`** (`if !capture.illuminations.is_empty() { skip }`). That guard is the thing to make rerun-aware, not any queue dedupe.
- **Task types are tiny structs** (`IlluminationTask{capture_id}`, `SparkTask{capture_ids}`, etc.) in `webhook/schema.rs`, each with a `TaskId::id() -> String`.
- **The `Beacon`** (`task/beacon.rs`) is the single funnel through which *all* task enqueues flow — the perfect choke point to also emit status events.
- **Deployment is a single Cloud Run service** (`cloudbuild.yaml` builds one image; `SERVICES` env var selects WebUI/API/Webhook). Tasks are queued via Cloud Tasks or Pub/Sub, so **the worker that completes a task may be a different process/instance than the one holding the user's HTTP connection.**
- **Frontend is HTMX 2.0.7 + one vanilla JS file** (`webui-v2.js`), no build step. You already have a custom XHR upload flow with progress UI.
- **Axum 0.8.9** (confirmed from `Cargo.lock` and the local crate source) ships a first-class SSE API: `axum::response::sse::{Event, Sse}`.

---

## 3. Why SSE (and not WebSockets or polling)

| Option | Pros | Cons | Fit |
|---|---|---|---|
| **SSE** | Unidirectional push over plain HTTP; works through proxies/Cloud Run; auto-reconnect built into browsers; **htmx-ext-sse handles it declaratively with ~zero JS** | One-way (fine — you only need server→client) | ✅ **Best fit** |
| WebSockets | Bidirectional | Needs a stateful upgrade, more JS, more server cruft, awkward through some proxies | ❌ Overkill |
| HTMX polling (`hx-trigger="every 2s"`) | Zero server work | Latency = poll interval; wasteful; still needs a "done?" endpoint | ⚠️ Fallback only |
| Long-polling | Simple | Reconnect churn, more complex server bookkeeping | ❌ |

**SSE is the idiomatic HTMX answer.** The htmx team maintains `htmx-ext-sse` specifically for this. The entire client-side surface is **three HTML attributes** — no custom JS for the transport itself:

```html
<body hx-ext="sse">
  <div sse-connect="/events" sse-swap="illumination-complete"></div>
</body>
```

That's it. The extension manages the `EventSource`, reconnects with exponential backoff, and swaps in whatever HTML the server sends for a named event.

---

## 4. Core architectural idea: a **task event stream** + **thin signals**

The cleanest, most flexible design separates two concerns:

1. **A server-side task-status source** that any code (webhook logic, admin backfill, rerun handlers) can publish task-status transitions to.
2. **A thin SSE endpoint** that subscribes a user's browser to that source, filtered by `user_id` **and** by the client's requested task types / captures.

Crucially, **SSE should carry *thin signals*, not full HTML.** This is the HATEOAS-friendly pattern that keeps the frontend cruft-free:

- SSE event: `{ task_type: "illumination", task_id: 123, status: "completed", capture_id: 123 }`
- Client reacts with a normal HTMX request to re-fetch the *partial* (`/detail/{id}` fragment or `/cards`), which the existing Tera templates already render.

This means:

- **No HTML-over-SSE** (which would duplicate template logic and bloat the stream).
- **One generic mechanism** for *all* task types (illumination, spark, search-index, ingest) — no bespoke channel per feature.
- **Reruns "just work"** because the event is keyed by `(task_type, task_id, run_id)` and the client just re-fetches whatever partial is relevant.

### 4.1 The task-status event shape

Every task-status transition is a small, typed struct. The SSE event name is constant (`event: task-status`); the payload carries the fields the client needs to route and react:

```rust
// src/events/mod.rs — the shape of a task-status transition
pub struct TaskStatusEvent {
    pub task_type: String,   // "illumination" | "spark" | "search_index" | "ingest"
    pub task_id: String,     // e.g. "123" or "123-456" for spark
    pub capture_id: i32,     // denormalized fan-out key (see §8.5)
    pub run_id: u64,         // distinguishes reruns
    pub status: task::Status, // Queued | InProgress | Completed | Error | ErrorFinal
    pub background: bool,    // true for backfill/bulk tasks (see §4.2)
    pub user_id: i32,        // for per-user filtering
}
```

The SSE wire format is a flat JSON object:

```json
{ "task_type": "illumination", "task_id": "123", "capture_id": 123, "status": "completed", "run_id": 1, "background": false }
```

**Why a single named SSE event (`task-status`) rather than per-status names (`illumination-complete`, etc.):** named events are fine for a fixed set, but they don't scale to "subscribe to a subset" or "route by status" cleanly. A single event name with a `status` field in the payload keeps the client logic uniform and lets it filter on `status`/`task_type`/`capture_id` as needed.

### 4.2 Client subscription filtering (points 2 & 3)

The client **explicitly registers the captures it cares about**. For `task_status`, there is **no "listen to everything" default** — the client must always send `capture_ids`. This is simpler, better, and more efficient:

1. **`capture_ids`** — **required** for `task_status`. The client lists the capture IDs it's currently rendering. Only events whose `capture_id` is in the list are delivered.
2. **`task_types`** — which task types to receive (e.g. `task_types=illumination,spark`). Defaults to all. This addresses point 2.

```http
GET /events?task_types=illumination,spark&capture_ids=123,456,789
```

The server filters on both `user_id` (always, for security) and the requested `capture_ids`/`task_types` (for relevance). The client **re-registers** its interests by reconnecting with new query params (see §8.6 for the adaptive-lifetime mechanism, which makes re-registration natural).

> **Why force explicit `capture_ids` for `task_status` (rather than a "listen to all" default)?**
> - **It's simpler.** No special-casing of "all vs. some" — the rule is uniform: *you get events for the captures you registered.*
> - **It's more efficient.** The server filters at the source, so the stream only carries events the page can actually use. No wasted bandwidth, no client-side filtering of irrelevant events.
> - **It fits the app's shape.** Because Dreamscroll is photo-heavy, a page renders only a bounded number of captures — images are render/memory heavy, so **a page will realistically never exceed a few hundred captures MAX**. Registering a few hundred IDs is trivial (a comma-separated query param), and it's far cheaper than streaming every task event for the user.
> - **It makes backfill tracking natural.** If you're watching a specific set of captures, you get their events — including background ones (see §4.3). No separate "opt in to backfill" mode needed.
>
> **The trade-off:** the client must keep its `capture_ids` list in sync with what's on screen (as the user scrolls, add/remove IDs). This is a small amount of JS, and it's exactly the kind of bookkeeping the adaptive-lifetime reconnect (§8.6) already makes natural — each reconnect re-registers the current set.

### 4.3 The `background` flag — backfill tracking is opt-in via `capture_ids`

A **`background` flag** on each task distinguishes bulk/backfill work from user-initiated work:

- **`background = false`** — user-initiated tasks (an upload, a rerun). These are the ones the user cares about.
- **`background = true`** — bulk/backfill tasks (e.g. an admin backfill that re-illuminates hundreds of captures). The user isn't watching these individually.

**The filtering semantics (with mandatory `capture_ids`):**

| Subscription | Behavior |
|---|---|
| `/events?capture_ids=123,456` | **Any** task updates (including `background`) **for those specific IDs**. You get backfill tracking when you're explicitly watching a set of captures. |

Because the client **always** registers `capture_ids`, there is no "listen to all" stream to spam. A backfill of hundreds of captures simply never reaches the page unless the page is explicitly tracking one of those captures. The `background` flag is still useful as **metadata** (the client can choose to render background tasks differently, e.g. a subtle "backfilling…" indicator), but it is **no longer needed as a filter** — the mandatory `capture_ids` already prevents backfill noise.

> **Design note:** the `background` flag remains persisted on the row. It's available for future use (e.g. an admin UI that wants to watch all backfill progress, or a `background=true` param if we ever add a "listen to everything" mode for a specific event type). For `task_status`, the mandatory `capture_ids` rule is the primary mechanism.

---

## 5. Server-side design

### 5.1 The canonical source of truth is the DB — not an in-process bus

**The in-process `tokio::sync::broadcast` idea is dropped.** It's fragile in Cloud Run for exactly the reason you flagged: the worker that completes a task can be a *different instance* than the one holding the user's SSE connection, so an in-memory channel on instance A would never see events published on instance B. Any design that relies on in-process state for correctness is wrong here.

**The `task_status` table (Postgres) is the single canonical source of truth.** Every status transition is a row write. The SSE handler reads from the DB. There is no separate in-memory event bus to keep in sync — the DB *is* the bus.

The remaining question is purely about **latency**: how does a connected browser learn about a new row *quickly* instead of waiting for a poll interval? Two mechanisms, used together:

1. **Postgres `LISTEN`/`NOTIFY`** — the idiomatic, dependency-free way to get cross-instance push. A worker writes the status row, then `NOTIFY`s a channel. Every instance's SSE handler holds a `LISTEN` connection and wakes up on the notification, then re-reads the row(s) from the DB. This gives near-real-time push across all instances with **zero new dependencies** (it's built into Postgres) and **no in-process state to drift**.
2. **A short poll fallback** — belt-and-suspenders. Even if `LISTEN/NOTIFY` is unavailable or a notification is missed, the client (or the SSE handler) can re-query the DB on a modest interval (e.g. every 5–10s) to reconcile. This guarantees eventual correctness even in the worst case.

> **Why `LISTEN/NOTIFY` and not the in-process bus:** the in-process bus only works when producer and consumer share a process. In Cloud Run they don't. `LISTEN/NOTIFY` is the *distributed* equivalent — it's the same "publish/subscribe" idea, but the channel lives in Postgres, which every instance already shares. It's the natural fit for "DB as canonical source of truth."

### 5.1a The task-status event model (DB-backed)

The event shape is the `TaskStatusEvent` from §4.1. It's persisted as a row (for replay) and delivered over SSE. The `capture_id` is the fan-out key the client uses to route a single SSE stream to the right card (see §8.5).

```rust
// src/events/mod.rs — the shape of a task-status transition
pub struct TaskStatusEvent {
    pub task_type: String,
    pub task_id: String,
    pub capture_id: i32,
    pub run_id: u64,
    pub status: task::Status,
    pub background: bool,   // true for backfill/bulk tasks (see §4.3)
    pub user_id: i32,
}
```

This maps 1:1 onto a `task_status` row (see §6).

### 5.2 Where task status gets written (and notified)

Task status is written at the natural choke points, each of which **writes a row and `NOTIFY`s**:

1. **In the `Beacon`** (`task/beacon.rs`) — every `signal_*` already funnels through here. Write a `Queued` row on enqueue. This gives "queued" status for free, everywhere, including admin backfill.
2. **In the webhook logic** (`logic/illuminate.rs`, `logic/spark.rs`, `logic/search_index.rs`) — write `InProgress` (start) and `Completed`/`Error` (finish). The `Beacon` is passed into `WebhookState` and the logic functions already receive `service_api`; add a small `StatusWriter` alongside.

A tiny helper encapsulates "write row + notify" so callers never touch the channel directly:

```rust
// src/events/status_writer.rs
pub struct StatusWriter { /* holds a DB connection + the notify channel name */ }

impl StatusWriter {
    pub async fn write(&self, event: &TaskStatusEvent) -> anyhow::Result<()> {
        // 1. UPSERT the task_status row (keyed by task_type, task_id, run_id)
        // 2. NOTIFY task_status_channel, '<task_id>'  (payload is just a hint)
    }
}
```

### 5.3 The SSE endpoint

A new route in `webui/v2/maker.rs` (protected by the same `login_required` layer as everything else — **auth is free**):

```rust
// src/webui/v2/r_events.rs
pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(params): Query<EventParams>,   // task_types=..., capture_ids=...
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let user_id = user.id;

    let stream = async_stream::stream::from_fn(async move |emit| {
        // 1. Replay recent task status for this user matching the subscription (see 5.4)
        // 2. Loop:
        //    a. Wait on the LISTEN channel (with a timeout) for a NOTIFY
        //    b. On notify (or timeout), re-query the DB for this user's matching rows
        //    c. Emit an Event for each
        // 3. Send a keep-alive comment periodically
        // 4. Enforce the adaptive lifetime (see 8.6)
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()).into_response())
}
```

The stream emits `Event::default().event("task-status").json_data(&event)` — a single named event whose payload carries the `status`/`task_type`/`capture_id` fields.

**Filtering is layered:**
- **`user_id`** — always, for security. Never leak one user's task status to another.
- **`capture_ids`** — **required** for `task_status`. Only events for the registered capture IDs are delivered (see §4.2).
- **`task_types`** — the client's requested task types (from query params).
- **`background`** — metadata on each event; not a filter for `task_status` (the mandatory `capture_ids` already prevents backfill noise). See §4.3.

Since the SSE connection is authenticated via the session cookie, `user_id` is known at connect time and used both in the DB query and in the emitted events.

### 5.4 The "replay on connect" problem (now trivial)

SSE is **ephemeral** — if the browser reconnects (which htmx-ext-sse does aggressively), it misses events that happened while disconnected. **Because the DB is the source of truth, replay is just a query:**

1. **On connect**, the SSE handler queries `task_status` for this `user_id` matching the requested `capture_ids`/`task_types`, where `status IN ('queued','in_progress')` (and optionally recent `completed`/`error`), and emits those rows immediately. The client is instantly reconciled with reality — no missed events, no cross-instance gap.
2. **On every `NOTIFY` (or poll tick)**, re-query the DB for this user's matching rows that changed since the last emission, and emit them.

There is **no in-memory state to lose** and **no cross-instance coordination problem** — the DB row is the single record, and both the producer (writer) and the SSE handler (reader) agree on it. This is the entire point of making the DB canonical.

### 5.5 Where the initial (clean-slate) status comes from

**For now, the `/events` "current status snapshot" is the canonical source of the initial task status.** The page-load HTML render does **not** include task status — that's a deliberate simplification for this phase, to be revisited later.

**The flow:**
1. **Page loads** → the HTML renders the captures (images, metadata) but **not** their task status. A capture that's mid-illumination simply shows no status pill yet.
2. **`/events` connects** → the handler's replay (§5.4) queries `task_status` for the registered `capture_ids` and emits the current in-flight statuses immediately. The client applies them (e.g. shows "illuminating…" on the matching cards).
3. **Subsequent updates** → SSE `task-status` events keep the status current as tasks transition.

**Why this is fine for now:**
- **It's simpler.** The page-load render doesn't need to join against `task_status` or render per-status states. The card template only needs to handle the *post-SSE* status rendering.
- **The snapshot is authoritative.** Because the `/events` replay reads the same `task_status` table, the initial status is correct at connect time — no render→connect race to worry about, because the snapshot *is* the current state.
- **The gap is tiny.** The only window where a card shows no status is between page load and the SSE connection opening (sub-second). For a personal app, that's acceptable.

> **Deferred (revisit later):** having the page-load HTML render also include task status (so the initial view is correct even before SSE connects, and works if SSE is unavailable). This is a clean, additive change later — the card template would render status from `task_status` at render time, and the `/events` replay would remain as the safety net. For now, the `/events` snapshot is canonical.

> **Concretely:** a capture that's mid-illumination (10s) shows no status pill on initial page load; the `/events` replay delivers `{ status: "in_progress" }` for it on connect, the client shows "illuminating…"; when the task completes, the SSE `task-status` event fires, the client re-fetches the card partial, and it re-renders as "done."

---

## 6. Task status persistence (the canonical source of truth)

`TaskQueue::get_status()` is unimplemented everywhere. Recommend a small, focused **`task_status` table** rather than trying to make each backend report status (Cloud Tasks and Pub/Sub don't give clean per-task status without extra plumbing). **This table is the source of truth** — the SSE handler reads it, the workers write it, and `LISTEN/NOTIFY` just tells readers "something changed, go look":

```sql
CREATE TABLE task_status (
    id            BIGSERIAL PRIMARY KEY,
    task_type     TEXT NOT NULL,          -- 'illumination' | 'spark' | ...
    task_id       TEXT NOT NULL,          -- capture_id or capture_ids joined
    run_id        BIGINT NOT NULL DEFAULT 1,  -- increments on rerun
    user_id       INT NOT NULL,
    status        TEXT NOT NULL,          -- queued|in_progress|completed|error|error_final
    attempts      INT NOT NULL DEFAULT 0,
    background    BOOLEAN NOT NULL DEFAULT false,  -- true for backfill/bulk tasks (see §4.3)
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (task_type, task_id, run_id)
);

-- The NOTIFY channel name (a constant in code, not a table)
-- SELECT pg_notify('task_status_channel', task_id) FROM ...;
```

- **Written by** the beacon (on enqueue) and the webhook logic (on start/finish) — each write is followed by a `NOTIFY task_status_channel`.
- **Read by** the SSE handler on connect (replay) and on every `NOTIFY`/poll tick (incremental), filtered by `user_id` + `task_types` + `capture_ids` + the `background` rule.
- **`run_id`** is the key to reruns (below).
- **`background`** marks bulk/backfill tasks so the default `/events` stream can ignore them (see §4.3).

> **Why a focused `task_status` table (not a generic `events` table):** task state is a first-class, non-trivial problem of its own — it has a lifecycle (`queued → in_progress → completed/error`), retries/attempts, and reruns. A dedicated table with typed columns (`task_type`, `task_id`, `run_id`, `status`, `attempts`) models that cleanly and is queryable. A generic `kind`+`payload` JSONB table would dilute this and make task-state queries awkward. **Capture lifecycle events are a separate, TBD concern** (see §13) and should get their own mechanism when we tackle them — not be shoehorned into `task_status`.

This also fixes a latent bug from the audit notes: *"No task retry/dead-letter in LocalTaskQueue — failed tasks silently dropped."* With a `task_status` table, `Error`/`ErrorFinal` states become observable.

### 6.1 `LISTEN/NOTIFY` mechanics and the connection budget

The one real constraint is **connection count**. The pool is capped at 5 (`max_connections(5)` in `database/postgres.rs`, sized for the `db-f1-micro` tier), and `LISTEN` requires a **dedicated, long-lived connection** (a pooled connection that returns to the pool would leak the `LISTEN` registration). So:

- **One dedicated `LISTEN` connection per instance** (not per SSE connection). All SSE handlers on an instance share it via a small fan-out: the `LISTEN` task receives notifications and forwards them to in-process `tokio::sync::broadcast` *receivers* (one per SSE connection). This is fine — the in-process channel is now only a *local delivery* mechanism for notifications that already arrived via Postgres, not the source of truth. It cannot drift because it's just echoing DB notifications.
- **Budget check:** 1 dedicated `LISTEN` connection per instance + the normal pool of 5. With Cloud Run scaling to a handful of instances, this stays well within the `f1-micro` connection limit. If the tier is ever raised, this becomes even less of a concern.
- **Fallback:** if a dedicated `LISTEN` connection can't be established (or to be extra safe), the SSE handler falls back to a **poll tick** (re-query the DB every N seconds). This keeps correctness with zero extra connections — just slightly higher latency.

> **Why not one `LISTEN` connection per SSE connection?** That would multiply connections by concurrent users and blow the `f1-micro` budget. Sharing one `LISTEN` per instance and fanning out locally is the right trade-off: the DB is still the source of truth, and the local channel is a pure delivery optimization with no correctness role.

---

## 7. Reruns ("Illuminate this again with a new model")

This is where the current design breaks, and it's worth calling out explicitly:

> The **idempotency guard in `logic/illuminate.rs`** (`if !capture.illuminations.is_empty() { skip }`) silently swallows a rerun because the capture already has an illumination. (Note: this is **not** `OneShotQueue` — that is dead code being removed. `LocalTaskQueue` does not dedupe.)

**The fix:** make the task identity include the *run*, not just the capture. Two options:

**Option A (recommended, minimal):** Add a `run_id` (or `model` + `force`) field to `IlluminationTask`:

```rust
pub struct IlluminationTask {
    pub capture_id: i32,
    pub run_id: u64,          // NEW — distinguishes reruns
    pub model: Option<String>, // NEW — which model to use
}
```

- `TaskId::id()` returns `"{capture_id}-{run_id}"` so each rerun is a distinct task.
- The `Beacon` gets a `signal_illumination(capture_id, run_id, model)` variant.
- `logic/illuminate.rs` **removes the idempotency guard** (`if !capture.illuminations.is_empty() { skip }`) when a rerun is requested — or better, inserts a *new* illumination row (the schema already supports multiple illuminations per capture; the template just renders `| first`).

**Option B:** A separate `rerun` task type. More moving parts; Option A is cleaner.

**The rerun UI** is then just a button on the detail page:

```html
<button hx-post="/detail/{{ capture.id }}/rerun"
        hx-vals='{"model": "gemini-2.5-pro"}'>
  Re-illuminate
</button>
```

The handler calls `beacon.signal_illumination(capture_id, next_run_id, model)`, which publishes a `Queued` event with the new `run_id`. The client's SSE listener sees it and shows "illuminating…" then re-fetches when `Completed` arrives.

---

## 8. Client-side design (minimal cruft, as requested)

### 8.1 One SSE connection per page

Add to the base templates (`index.html.tera`, `detail.html.tera`):

```html
<body hx-ext="sse">
  <!-- thin signal listener: re-fetch the relevant partial on completion -->
  <div sse-connect="/events?task_types=illumination,spark,search_index,ingest&capture_ids={{ visible_capture_ids }}"
       sse-swap="task-status"
       hx-get="/detail/{{ capture.id }}"
       hx-target="#card-feed"
       hx-swap="innerHTML">
  </div>
</body>
```

**Subtlety:** `sse-swap` swaps the SSE *data* into the element, but we want to *trigger a re-fetch* instead. The htmx-ext-sse docs give exactly the right tool for this: **`hx-trigger="sse:<event>"`** for callbacks, and `sse-swap` for direct content swap. So the idiomatic pattern is:

```html
<body hx-ext="sse">
  <div sse-connect="/events?task_types=illumination,spark,search_index,ingest&capture_ids={{ visible_capture_ids }}"></div>

  <!-- On a task-status event, re-fetch the detail partial -->
  <div hx-get="/detail/{{ capture.id }}"
       hx-trigger="sse:task-status"
       hx-target="#card-feed"
       hx-swap="innerHTML">
  </div>
</body>
```

This is **pure HTML** — no JS. The server sends a single named event `task-status` whose payload carries the `status`/`task_type`/`capture_id` fields; htmx fires a GET to re-render the partial, and the existing Tera templates do the rest. This is the HATEOAS pattern: **SSE says "something changed", HTMX fetches the new state.**

> **Note on `capture_ids` in the URL:** `capture_ids` is **required** for `task_status` (see §4.2). The template injects the page's visible capture IDs (`{{ visible_capture_ids }}`). Because the connection is re-established on reconnect (and on the adaptive-lifetime cycle in §8.6), the client naturally re-registers its interests each time. The page can also update the `sse-connect` URL dynamically (via JS) as the set of visible captures changes (e.g. on scroll) — see §8.6.

### 8.2 A small status indicator (optional, still no JS)

Show a live "illuminating…" state with a second listener that swaps in a tiny status partial:

```html
<div sse-connect="/events?task_types=illumination&capture_ids={{ visible_capture_ids }}">
  <div sse-swap="task-status">
    <span class="status-pill">queued</span>
  </div>
</div>
```

The server sends small HTML fragments for the relevant `task-status` payloads. This keeps the "live status" feel without any imperative JS.

### 8.3 What about the upload flow?

The upload already uses a custom XHR with progress. After upload completes, the server returns `{capture_id, detail_url}`. The client can **immediately open the SSE connection scoped to that capture** (or just rely on the page-wide `/events` connection) and show "Illuminating…" until the `illumination-complete` event arrives, then re-fetch. Since the page already has `/events` connected, **zero new JS** — just the existing `showUploadNotice` logic extended to also listen for the completion event.

---

## 8.5 One SSE channel, many cards — how a single page-level connection relays N tasks

**The question:** the timeline/home page can render 50+ capture cards, and a user can upload 5 screenshots in quick succession so all 5 are queued/illuminating at once. How does *one* page-level SSE channel relay the status of *all 5*?

**The answer: the SSE channel is a *multiplexed bus*, not a per-card connection.** There is exactly **one** `EventSource` per page (one TCP/HTTP connection to `/events`). It carries a *stream of many named events*, each tagged with which capture it belongs to. The client fans that single stream out to the right card. Nothing about the number of cards or concurrent tasks changes the connection count — it's always 1.

### How the multiplexing works

**Server side** — every task-status transition publishes an event onto the bus with a `capture_id` (and `task_type`/`task_id`/`run_id`) in the payload. The SSE handler just forwards *all* of that user's events down the one connection. It does not care how many distinct captures are in flight:

```
5 uploads → 5 IlluminationTasks → 5× (Queued → InProgress → Completed) events
                                                          │
                                                          ▼
                              one /events connection, 15 events, each tagged with capture_id
```

**Client side** — the key insight is that **htmx-ext-sse dispatches each SSE event as a DOM `CustomEvent` on the element that declares the listener**, and the event's `detail` carries the raw SSE `data`. So a card can listen for *its own* completion by filtering on the payload's `capture_id`.

### The two idiomatic client patterns

**Pattern A — one listener per card, filtered by `capture_id` (recommended for the timeline).** Each card carries a tiny listener that reacts only when the event's `capture_id` matches its own:

```html
<!-- inside each card, e.g. #card-{{ capture.id }} -->
<div class="card"
     id="card-{{ capture.id }}"
     hx-get="/cards/{{ capture.id }}"
     hx-trigger="sse:task-status"
     hx-swap="outerHTML"
     data-capture-id="{{ capture.id }}">
</div>
```

Because the server tags every event with `capture_id`, the card's `hx-trigger="sse:task-status"` fires for *every* task-status event — but the re-fetch is scoped to that card's own URL (`/cards/{{ capture.id }}`), so it only re-renders itself. The `data-capture-id` attribute is there for the optional JS status-pill path (below) to filter precisely.

> **Important subtlety:** `hx-trigger="sse:task-status"` fires on *every* task-status event, not just this card's. That's fine here because the re-fetch URL is per-card — a card re-fetching its own partial when *another* card changes is harmless (it just re-renders the same content). If you want to avoid even that, use the JS filter in Pattern B.

**Pattern B — a single JS listener that routes by `capture_id` (for precise fan-out / status pills).** One listener on the shared connection reads `event.detail`, checks `capture_id`, and updates only the matching card:

```js
// webui-v2.js — one listener, routes to the right card
document.body.addEventListener('sse:task-status', (e) => {
  const data = JSON.parse(e.detail.data);   // { task_type, task_id, capture_id, status, run_id }
  const card = document.querySelector(`#card-${data.capture_id}`);
  if (card) {
    card.classList.remove('is-illuminating');
    htmx.ajax('GET', `/cards/${data.capture_id}`, { target: card, swap: 'outerHTML' });
  }
});
```

This is the *precise* version: it touches only the card whose `capture_id` matches, so 5 concurrent tasks update 5 distinct cards independently, in any completion order.

### Why this stays simple

- **Connection count is constant (1)** regardless of cards/tasks — no N+1 connections, no per-card `EventSource` churn.
- **The server is dumb and generic** — it just forwards tagged task-status events; it has no per-card logic.
- **The client is either pure-HTML (Pattern A)** or a ~10-line JS router (Pattern B). Both are far simpler than 5 separate connections.
- **Ordering is naturally handled** — each event carries its own `capture_id`, so cards update independently and out-of-order completions are fine.

### The one thing to get right: the event payload must carry `capture_id`

For the fan-out to work, every event must be self-describing. The `TaskStatusEvent` in §5.1a already denormalizes `capture_id` for exactly this reason. If a task type ever has no single capture (e.g. a spark over many captures), the payload carries the relevant ids and the client routes on whatever key is appropriate — the mechanism is identical.

---

## 8.6 Adaptive connection lifetime (point 4 — the biggest concern)

**The problem:** our topology is narrow (see `topology_and_throughput.md`). Each open SSE connection occupies a Cloud Run HTTP concurrency slot for its entire duration, and on `db-f1-micro` the DB connection total is also tight. Holding connections open indefinitely when idle is wasteful and risks exhausting the budget as users accumulate.

**The strategy: a dynamic, adaptive lifetime.** By default the page listens to `/events` for **5 minutes**, and the connection **automatically extends** whenever:
- **(a) the user does something** (any interaction — a click, an HTMX request, an upload, a scroll-triggered fetch), or
- **(b) something meaningful happens on the server** (an event is delivered).

If neither happens for 5 minutes, the connection **closes gracefully** and the page falls back to on-demand refresh (the pre-SSE behavior). The next user action reopens it.

### Why this works

- **Idle pages don't hold connections forever.** A user who opens the timeline and walks away releases the slot after 5 min of inactivity. This directly addresses the "N open tabs pin N concurrency slots" concern.
- **Active pages stay live.** As long as the user is interacting or events are flowing, the connection keeps extending — so the real-time experience is uninterrupted during actual use.
- **It's a natural fit for SSE.** SSE is designed to be re-established; htmx-ext-sse auto-reconnects. Closing after idle is just a graceful `sse-close` / server-side stream end, and the next interaction reconnects.

### How it's implemented

**Server side** — the SSE handler tracks two timestamps:
- `last_activity` — updated on every delivered event (condition b).
- `last_client_touch` — updated when the client signals activity (condition a).

The stream ends when `now - max(last_activity, last_client_touch) > IDLE_TIMEOUT` (5 min). It sends a final `event: task-status` with `{ status: "idle_close" }` (or a dedicated `sse-close` event) so the client knows the close was intentional, not an error.

**Client side** — two mechanisms keep the connection alive while active:

1. **Server events extend it automatically** (condition b) — no client work needed.
2. **Client activity extends it** (condition a) — the client sends a lightweight "heartbeat" on user interaction. The cleanest way: piggyback on the existing HTMX request cycle. Every HTMX request already hits the server; the SSE handler can't see those directly, but the client can send a tiny `POST /events/heartbeat` (or reuse an existing endpoint) on interaction. Simpler still: since the page reconnects on the next action anyway, the client can just **reconnect** (re-issue `sse-connect`) on user activity rather than maintaining a heartbeat — the reconnect itself resets the 5-min timer.

> **Recommended (simplest):** rely on **server events** to extend the lifetime during active work, and let the client **reconnect on user interaction**. No heartbeat endpoint needed. The flow:
> - Page loads → opens `/events` (5-min timer starts).
> - User interacts → htmx fires a request → on response, the client reconnects `/events` (fresh 5-min timer). This is a few lines in `webui-v2.js` (listen for `htmx:afterRequest` / `htmx:afterOnLoad` and re-issue the SSE connect).
> - Server event arrives → timer resets server-side.
> - 5 min of neither → server closes the stream; page is now static until the next interaction.

### Re-registration on reconnect (ties into points 2 & 3)

Because the connection is re-established on every reconnect, the client **re-registers its subscription each time** — the `sse-connect` URL carries the current `task_types` and `capture_ids`. Since `capture_ids` is **required** for `task_status` (§4.2), this re-registration is essential: as the user scrolls and the set of visible cards changes, the page updates the `sse-connect` URL to track only what's on screen. The adaptive lifetime and the mandatory-`capture_ids` subscription model reinforce each other.

### Interaction with the topology budget

- **Idle connections are released** after 5 min → concurrency slots free up.
- **Active connections are bounded** to actual use → no unbounded accumulation.
- **Reconnect is cheap** (SSE + htmx-ext-sse handle it) and the DB replay (§5.4) reconciles any missed events on reconnect.

> **Caveat:** the 5-min idle timeout must be **shorter than the Cloud Run request timeout** (default 5 min, max 60 min). If the service timeout is left at the 5-min default, an SSE connection that's been idle for 5 min would be killed by Cloud Run anyway — so the adaptive close should happen *before* that, or the service timeout must be raised. Set the service timeout to e.g. 15 min and let the adaptive 5-min idle close happen first. (See `topology_and_throughput.md` §4.)

---

## 9. Deployment / multi-instance correctness

Because the worker can be a different instance than the browser's connection, **the DB is the source of truth and `LISTEN/NOTIFY` is the cross-instance push**:

1. **Worker (any instance)** writes the task-status row, then `NOTIFY`s `task_status_channel`.
2. **Every instance** runs one dedicated `LISTEN` connection. On a notification, it fans out locally to its connected SSE handlers, which re-query the DB for that user's matching rows and emit them.
3. **Replay on connect** reconciles any missed events — the SSE handler queries the DB for the user's matching task status on connect, so a browser that reconnects (or connects to a different instance) is instantly correct.
4. **Poll fallback** guarantees eventual correctness even if `LISTEN/NOTIFY` is unavailable — the SSE handler re-queries the DB on a modest interval.

There is **no in-process state that can drift or be lost** — the local channel is only a delivery optimization for notifications that already arrived from Postgres.

For Cloud Run specifically: SSE works fine through the Cloud Run ingress as long as the service **doesn't set a short request timeout** (SSE is a long-lived request). Cloud Run's default request timeout is 60s for the *first* byte, but a streaming response that keeps sending keep-alives is fine. Set the service timeout appropriately (e.g. 15 min) and rely on `KeepAlive::default()` to keep the connection alive. **The adaptive 5-min idle close (§8.6) must happen before the service timeout**, so the connection ends gracefully rather than being killed by Cloud Run.

---

## 10. Proposed file changes (summary)

| File | Change |
|---|---|
| `src/events/mod.rs` *(new)* | `TaskStatusEvent` struct + `StatusWriter` (write row + `NOTIFY`) |
| `src/events/notifier.rs` *(new)* | dedicated `LISTEN` connection + local fan-out to SSE receivers |
| `src/task/taskqueue.rs` | (optional) add `run_id` awareness to `TaskId`/status |
| `src/task/beacon.rs` | write `Queued` status + `NOTIFY`; add `signal_illumination(capture_id, run_id, model)`; accept a `background` flag |
| `src/webhook/schema.rs` | add `run_id`/`model` to `IlluminationTask`; update `TaskId::id()` |
| `src/webhook/logic/illuminate.rs` | write `InProgress`/`Completed`/`Error` + `NOTIFY`; honor rerun (drop idempotency guard on rerun) |
| `src/webhook/logic/spark.rs`, `search_index.rs` | write status + `NOTIFY` |
| `src/webhook/webhook_state.rs` | add `StatusWriter` |
| `src/webui/v2/maker.rs` | add `/events` SSE route; thread `StatusWriter` + notifier into `WebState` |
| `src/webui/v2/r_events.rs` *(new)* | SSE handler (replay from DB, listen for notifications, filter by user + task_types + capture_ids + background rule, adaptive lifetime) |
| `src/webui/v2/r_rerun.rs` *(new)* | rerun endpoint |
| `src/database/` | `task_status` table (incl. `background` column) + SeaORM model |
| `web/v2/templates/*.tera` | add `hx-ext="sse"`, `sse-connect` (with `task_types`/`capture_ids`), `hx-trigger="sse:task-status"`; render per-status card state (queued/in_progress/completed/error) applied from SSE events (§5.5) |
| `web/v2/static/webui-v2.js` | extend upload notice to react to task-status events; reconnect `/events` on user interaction (adaptive lifetime) |

---

## 11. Why this is "simple, idiomatic, robust, flexible"

- **Simple:** The client is ~3 HTML attributes. The server uses Postgres `LISTEN/NOTIFY` (built into Postgres, zero new dependencies) + one small `StatusWriter`. No build step, no JS framework.
- **Idiomatic:** SSE is the canonical HTMX companion; `htmx-ext-sse` is the official extension. Axum has first-class SSE support. `LISTEN/NOTIFY` is the idiomatic Postgres pub/sub. The `TaskQueue::get_status()` trait already anticipated this.
- **Robust:** The `task_status` table is the **single canonical source of truth** — it survives reconnects, restarts, and multi-instance workers with no in-process state to drift. `LISTEN/NOTIFY` gives low latency; the poll fallback guarantees correctness; keep-alives + auto-reconnect handle flaky connections. The adaptive lifetime keeps idle connections from accumulating.
- **Flexible:** The `(task_type, task_id, run_id)` task-status model is generic — illumination, spark, search-index, ingest all flow through the same table/channel. Adding a new task type = write a row + `NOTIFY` + add an `hx-trigger="sse:task-status"` line. Client subscription (`task_types` + `capture_ids`) keeps the stream relevant. Reruns are a first-class concept via `run_id`.

---

## 12. Suggested implementation order

1. **`task_status` table + model** (foundation; also fixes the "no retry observability" audit note). Include the `background` column.
2. **`StatusWriter`** (write row + `NOTIFY`) + wire into `Beacon` and `WebhookState`. Thread the `background` flag through the beacon signals.
3. **`/events` SSE route** with user filtering + DB replay + poll fallback + subscription params (`task_types`, `capture_ids`) + the `background` rule.
4. **Client wiring** (`hx-ext="sse"`, `sse-connect`, `hx-trigger="sse:task-status"`) — live updates for the *existing* upload flow should appear immediately.
5. **Adaptive lifetime** (5-min idle close + reconnect-on-interaction) — the topology safeguard.
6. **Rerun support** (`run_id`/`model` on `IlluminationTask`, rerun endpoint, drop idempotency guard on rerun).
7. **Extend to spark/search-index** as the pattern proves out.

---

## 13. Open questions / follow-ups

- **SSE payload format:** thin JSON signals (recommended) vs. small HTML fragments for direct `sse-swap`. The design supports both; pick per use-case.
- **`task_status` retention:** add a cleanup/eviction policy for old `run_id`s to avoid unbounded table growth.
- **Cloud Run timeout:** confirm the service-level request timeout is set high enough for long-lived SSE connections (and above the 5-min adaptive idle close).
- **Multiple illuminations per capture:** decide whether reruns replace the existing illumination or append a new one (schema supports both; template currently renders `| first`).
- **`background` flag semantics:** for now the `background` flag is metadata only — the mandatory `capture_ids` rule already prevents backfill noise (see §4.3). Revisit later — e.g. a `background=true` query param to opt in, or surfacing backfill progress in an admin UI. The flag is persisted, so it's available for any future use.
- **`capture_ids` is required for `task_status`:** the client always registers the captures it's tracking (see §4.2). This is simpler and more efficient than a "listen to all" default, and fits the app's bounded page size (a few hundred captures max). The client must keep its `capture_ids` list in sync with what's on screen (via the adaptive-lifetime reconnect, §8.6).
- **Reconnect-on-interaction cost:** confirm that re-issuing `sse-connect` on every HTMX request is cheap enough (it should be — SSE reconnect is lightweight and the DB replay reconciles state).
- **Initial-state source (resolved in §5.5):** for now, the `/events` "current status snapshot" is the canonical source of initial task status; the page-load HTML render does **not** include task status. Revisit later — having the page-load render also include status (so the initial view is correct before SSE connects / if SSE is unavailable) is a clean, additive change.
- **TBD — capture lifecycle events (created/deleted elsewhere):** explicitly out of scope for this phase. When we tackle it, it should get its **own mechanism** (likely a separate table/channel or a deliberate extension), not be shoehorned into `task_status`. The `capture_ids` subscription param and the single-SSE-connection-per-page design (§8.5) are forward-compatible with adding a second event type later.