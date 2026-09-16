# Real-time Task Status via SSE — Design

**Date:** 2026-09-15 (updated from 2026-09-14)
**Status:** Partially implemented. The task framework (`Task`/`TaskEnvelope`/`TaskQueue`/`TaskMaster`/`TaskStatusTracker`/`StatusCode`), task identity, status tracking, the **retry/exhaustion policy**, and the **entity-scoped query API** are **done and wired end-to-end** (submit → `Queued`; worker → `InProgress` → `Completed`/`ErrorWillRetry`/`ErrorExhausted`). The SSE delivery layer (`StatusListener`, `LISTEN/NOTIFY`, `/events` route, client wiring) is **not yet implemented** — see §12.
**Scope:** Relay accurate, up-to-date, low-latency **background-task status** to HTMX clients. The `task_status` table is a **first-class citizen** — focused purely on the task-state problem, which is non-trivial on its own. Signaling/tracking **capture lifecycle events** (created/deleted elsewhere) is explicitly **TBD** and out of scope for this phase.

> **See also:** `_project/plans/pragmatism.md` — the ledger of trade-offs we are deliberately tolerating (concurrency races, cost/throughput issues, deferred hardening) in order to focus on validating the app. Several items in §13 below are recorded there with revisit triggers rather than being fixed now.

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
  └─ insert_capture() → task_master.submit_illumination(user_id, IlluminationTask)
       └─ /_wh/cloudtask/illuminate → logic/illuminate::exec
            ├─ illuminate_capture()        (idempotent; skips if already illuminated)
            └─ logic/search_index::exec    (idempotent; skips if already indexed)
                 └─ insert_illumination()  ← row written, status now tracked
```

> **Note (2026-09-16):** `IngestTask` and `logic/ingest.rs` were **removed**. Illumination has no real purpose without search indexing, so `logic/illuminate::exec` now runs **both** steps as a single unit of work. The capture-create path calls `submit_illumination` directly. `r_illuminate` + the illumination queue are the live path; the `/_wh/cloudtask/illuminate` route is currently unused and reserved for future backfill / re-run flows.

### 2.2 Key facts that shape the design

- **`Task` is a trait, not an enum.** Each concrete task type (`IlluminationTask`, `SparkTask`, `SearchIndexTask`) lives in `src/logic/*.rs` and implements `task::Task`. The trait carries the task's **identity**: `task_type() -> &'static str`, `entity_type() -> &'static str` (e.g. `"capture"`, `"spark"`), and `entity_id(&self) -> i32`. A `TaskEnvelope<T>` wraps a task with `user_id`, `envelope_id`, and the payload (`task: Option<T>`).
- **Task identity is deterministic and lives in the envelope.** `TaskEnvelope::new(user_id, task, run)` builds `envelope_id = "u{user_id}-{task_type}-{entity_type}{entity_id}"` (e.g. `u1-illuminate-capture123`) and carries a 1-based `run`. There is **no UUID** and no separate `task_id.rs` — the old `make_task_id` free function was deleted. Because the id is deterministic, re-submitting the same logical work targets the *same logical task*; the `run` distinguishes one attempt from the next (see §7).
- **`TaskQueue<T>` is enqueue-only and generic.** `TaskQueue::get_status()` was removed (it was `unimplemented!()` everywhere); status lives in the `task_status` table. The trait is `async fn enqueue(&self, wrapped: TaskEnvelope<T>)`. There are two backends: `LocalTaskQueue` (in-process mpsc + semaphore) and `CloudTaskQueue` (Google Cloud Tasks). **Pub/Sub support was removed** (2026-09-15) to focus on Cloud Tasks.
- **`OneShotQueue` is dead code and will be removed.** It was an old local-only emulation of task execution and is **not used anywhere in production** — it appears only in `src/common/mod.rs` (module decl + re-export) and its own file `src/common/oneshotqueue.rs` (definition + unit tests). The `LocalTaskQueue` (in-process mpsc + semaphore) is the real local backend and does **not** dedupe. So the rerun problem is *not* caused by `OneShotQueue`; it's caused by the **idempotency guard in `logic/illuminate.rs`** (`if !capture.illuminations.is_empty() { skip }`). That guard is the thing to make rerun-aware, not any queue dedupe.
- **`TaskMaster`** (`task/taskmaster.rs`) is the single funnel through which *all* task enqueues flow — the perfect choke point to also record status. It replaced the old `Beacon` (removed). `TaskMaster` owns the backend queues **and** the `task_status` table, exposing `submit_*` / `begin_attempt` / `finish_attempt` / `query_*`. It is **not `Clone`** — it's shared via `Arc<TaskMaster>`. `StatusListener` (`task/status_listener.rs`) is the future `LISTEN`/`NOTIFY` thread (stub for now). **These two are the only structs that touch `task_status` directly.**
- **Status transitions are not a raw setter.** `TaskMaster::update_status` is **private**; workers must go through `begin_attempt` (reads the persisted attempt count, increments, writes `InProgress`, returns the 1-based attempt number) and `finish_attempt` (writes the outcome and returns an `AttemptOutcome` that drives the HTTP response). This keeps `attempts` and the retry decision consistent with the recorded status.
- **`TaskStatusTracker`** (`task/status_tracker.rs`) owns all `task_status` persistence (upsert keyed by `envelope_id`). `StatusCode` (`task/status_code.rs`) is the strongly-typed status enum; the DB stores only its integer discriminant (`status_code INT`).
- **Deployment is a single Cloud Run service** (`cloudbuild.yaml` builds one image; `SERVICES` env var selects WebUI/API/Webhook). Tasks are queued via Cloud Tasks, so **the worker that completes a task may be a different process/instance than the one holding the user's HTTP connection.**
- **Frontend is HTMX 2.0.7 + one vanilla JS file** (`webui-v2.js`), no build step. You already have a custom XHR upload flow with progress UI.
- **Axum 0.8.9** (confirmed from `Cargo.lock` and the local crate source) ships a first-class SSE API: `axum::response::sse::{Event, Sse}`.

---

## 3. Why SSE (and not WebSockets or polling)

| Option                                 | Pros                                                                                                                                                              | Cons                                                                               | Fit             |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | --------------- |
| **SSE**                                | Unidirectional push over plain HTTP; works through proxies/Cloud Run; auto-reconnect built into browsers; **htmx-ext-sse handles it declaratively with ~zero JS** | One-way (fine — you only need server→client)                                       | ✅ **Best fit**  |
| WebSockets                             | Bidirectional                                                                                                                                                     | Needs a stateful upgrade, more JS, more server cruft, awkward through some proxies | ❌ Overkill      |
| HTMX polling (`hx-trigger="every 2s"`) | Zero server work                                                                                                                                                  | Latency = poll interval; wasteful; still needs a "done?" endpoint                  | ⚠️ Fallback only |
| Long-polling                           | Simple                                                                                                                                                            | Reconnect churn, more complex server bookkeeping                                   | ❌               |

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

- SSE event: `{ task_type: "illumination", envelope_id: "u1-illuminate-capture123", entity_type: "capture", entity_id: 123, status: "completed" }`
- Client reacts with a normal HTMX request to re-fetch the *partial* (`/detail/{id}` fragment or `/cards`), which the existing Tera templates already render.

This means:

- **No HTML-over-SSE** (which would duplicate template logic and bloat the stream).
- **One generic mechanism** for *all* task types (illumination, spark, search-index, ingest) — no bespoke channel per feature.
- **Reruns "just work"** — the event is keyed by `(envelope_id, run)` and the client just re-fetches whatever partial is relevant. (See §7.)

### 4.1 The task-status event shape

Every task-status transition is a small, typed struct. The SSE event name is constant (`event: task-status`); the payload carries the fields the client needs to route and react:

```rust
// src/events/mod.rs — the shape of a task-status transition
pub struct TaskStatusEvent {
    pub task_type: String,   // "illumination" | "spark" | "search_index" | "ingest"
    pub envelope_id: String, // e.g. "u1-illuminate-capture123" (see §2.2)
    pub entity_type: String, // "capture" | "spark"
    pub entity_id: i32,      // the entity this task operates on (fan-out key, see §8.5)
    pub status: task::StatusCode, // Queued | InProgress | Completed | ErrorWillRetry | ErrorExhausted
    pub attempts: i32,       // 1-based attempt number of the latest attempt
    pub user_id: i32,        // for per-user filtering
}
```

The SSE wire format is a flat JSON object:

```json
{ "task_type": "illumination", "envelope_id": "u1-illuminate-capture123", "entity_type": "capture", "entity_id": 123, "status": "completed", "attempts": 1 }
```

> **Note on runs:** the design previously carried a `run_id` inside the task payload. `run` now lives on the `TaskEnvelope` and the `task_status` row (not on `StatusCode`, and not inside the task), so re-submitting the same logical work starts a **new run** rather than overwriting the previous row. See §7.

**Why a single named SSE event (`task-status`) rather than per-status names (`illumination-complete`, etc.):** named events are fine for a fixed set, but they don't scale to "subscribe to a subset" or "route by status" cleanly. A single event name with a `status` field in the payload keeps the client logic uniform and lets it filter on `status`/`task_type`/`entity_id` as needed.

### 4.2 Client subscription filtering (points 2 & 3)

The client **explicitly registers the captures it cares about**. For `task_status`, there is **no "listen to everything" default** — the client must always send `capture_ids`. This is simpler, better, and more efficient:

1. **`capture_ids`** — **required** for `task_status`. The client lists the capture IDs it's currently rendering. Only events whose `entity_id` is in the list are delivered.
2. **`task_types`** — which task types to receive (e.g. `task_types=illumination,spark`). Defaults to all. This addresses point 2.

```http
GET /events?task_types=illumination,spark&capture_ids=123,456,789
```

The server filters on both `user_id` (always, for security) and the requested `capture_ids`/`task_types` (for relevance). The client **re-registers** its interests by reconnecting with new query params (see §8.6 for the adaptive-lifetime mechanism, which makes re-registration natural).

> **How `capture_ids` maps to the DB:** the query param is a client-facing convenience. Internally it becomes `entity_type = 'capture' AND entity_id IN (...)`, matching the `task_status` columns (see §6). The SSE handler translates the subscription into the entity-scoped query (`query_incomplete_for_entity` per capture, or an `IN` variant).

> **Why force explicit `capture_ids` for `task_status` (rather than a "listen to all" default)?**
> - **It's simpler.** No special-casing of "all vs. some" — the rule is uniform: *you get events for the captures you registered.*
> - **It's more efficient.** The server filters at the source, so the stream only carries events the page can actually use. No wasted bandwidth, no client-side filtering of irrelevant events.
> - **It fits the app's shape.** Because Dreamscroll is photo-heavy, a page renders only a bounded number of captures — images are render/memory heavy, so **a page will realistically never exceed a few hundred captures MAX**. Registering a few hundred IDs is trivial (a comma-separated query param), and it's far cheaper than streaming every task event for the user.
> - **It makes backfill tracking natural.** If you're watching a specific set of captures, you get their events. No separate "opt in to backfill" mode needed.
>
> **The trade-off:** the client must keep its `capture_ids` list in sync with what's on screen (as the user scrolls, add/remove IDs). This is a small amount of JS, and it's exactly the kind of bookkeeping the adaptive-lifetime reconnect (§8.6) already makes natural — each reconnect re-registers the current set.

### 4.3 Backfill / bulk tasks — deferred

An earlier revision of this design carried a **`background` flag** on each task to distinguish bulk/backfill work from user-initiated work. **That field has been removed** (2026-09-16) — it was never populated (`TaskStatusTracker::record` hardcoded `false`) and backfill deserves its own design pass rather than a speculative column.

**Why it isn't needed for this phase:** the mandatory `capture_ids` subscription (§4.2) already prevents backfill noise. A backfill of hundreds of captures simply never reaches a page unless that page is explicitly tracking one of those captures. So the flag was never load-bearing as a filter.

> **Deferred:** backfill/bulk-task handling (marking tasks as background, surfacing backfill progress, an admin progress view) will be tackled in a dedicated plan-and-branch session. When it is, the natural shape is a new column on `task_status` plus a subscription param — but that decision is deliberately out of scope here.

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

The event shape is the `TaskStatusEvent` from §4.1. It's persisted as a row (for replay) and delivered over SSE. The `entity_id` is the fan-out key the client uses to route a single SSE stream to the right card (see §8.5).

```rust
// src/events/mod.rs — the shape of a task-status transition
pub struct TaskStatusEvent {
    pub task_type: String,
    pub envelope_id: String,
    pub entity_type: String,
    pub entity_id: i32,
    pub status: task::StatusCode,
    pub attempts: i32,
    pub user_id: i32,
}
```

This maps 1:1 onto a `task_status` row (see §6). The `StatusCode` enum lives in the task module (`task::StatusCode`); the DB stores only its integer discriminant (`status_code INT`).

### 5.2 Where task status gets written (and notified)

Task status is written at the natural choke points, each of which **writes a row and `NOTIFY`s**:

1. **In `TaskMaster::submit_*`** (`task/taskmaster.rs`) — every task enqueue funnels through here. It records a `Queued` row on enqueue. This gives "queued" status for free, everywhere, including admin backfill.
2. **In the webhook handlers** (`webhook/r_illuminate.rs`, `r_spark.rs`, `r_search_index.rs`) — each handler deserializes a `TaskEnvelope<T>`, then calls `begin_attempt` (writes `InProgress` + the incremented attempt number) and `finish_attempt` (writes `Completed`/`ErrorWillRetry`/`ErrorExhausted` and returns the `AttemptOutcome` that decides the HTTP status) around the `logic/*::exec` call. The `TaskMaster` is threaded through `WebhookState` as `Arc<TaskMaster>`.

> **Note:** status is written in the **webhook handler**, not inside `logic/*::exec`. The `logic` functions stay pure (they take the bare task and don't know about task identity/status). The handler owns the envelope and reports status around the `exec` call.

> **Note:** `TaskMaster::update_status` is **private**. The raw setter is deliberately not exposed, so callers cannot write a status that disagrees with the attempt count or the retry decision. See §6.1 for the retry/exhaustion policy.

A tiny helper encapsulates "write row + notify" so callers never touch the channel directly:

```rust
// src/events/status_writer.rs
pub struct StatusWriter { /* holds a TaskMaster (or DB conn) + the notify channel name */ }

impl StatusWriter {
    pub async fn write(&self, event: &TaskStatusEvent) -> anyhow::Result<()> {
        // 1. UPSERT the task_status row (keyed by envelope_id)
        // 2. NOTIFY task_status_channel, '<envelope_id>'  (payload is just a hint)
    }
}
```

> **Note:** `TaskMaster::update_status` already does the UPSERT (step 1). The `StatusWriter` is a thin wrapper that adds the `NOTIFY` (step 2) — or `TaskMaster` itself can own the notify. Either way, the two-owner rule holds: only `TaskMaster`/`StatusListener` touch `task_status`.

> **Note:** the `NOTIFY` is **not yet implemented**. Today `TaskMaster` writes the row only; the `NOTIFY` is the remaining piece of this step (see §12).

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

The stream emits `Event::default().event("task-status").json_data(&event)` — a single named event whose payload carries the `status`/`task_type`/`entity_id` fields.

**Filtering is layered:**
- **`user_id`** — always, for security. Never leak one user's task status to another.
- **`capture_ids`** — **required** for `task_status`. Only events for the registered capture IDs are delivered (see §4.2).
- **`task_types`** — the client's requested task types (from query params).

Since the SSE connection is authenticated via the session cookie, `user_id` is known at connect time and used both in the DB query and in the emitted events.

### 5.4 The "replay on connect" problem (now trivial)

SSE is **ephemeral** — if the browser reconnects (which htmx-ext-sse does aggressively), it misses events that happened while disconnected. **Because the DB is the source of truth, replay is just a query:**

1. **On connect**, the SSE handler queries `task_status` for this `user_id` matching the requested `entity_type`/`entity_id`/`task_types`, using the **incomplete** predicate (`status_code IN (Queued, InProgress, ErrorWillRetry, ErrorExhausted)` — i.e. everything except `Completed`), and emits those rows immediately. The client is instantly reconciled with reality — no missed events, no cross-instance gap. See §6.1 for why `ErrorExhausted` is included and `Completed` is not.
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

`TaskQueue::get_status()` was removed (it was unimplemented everywhere). Recommend a small, focused **`task_status` table** rather than trying to make each backend report status (Cloud Tasks doesn't give clean per-task status without extra plumbing). **This table is the source of truth** — the SSE handler reads it, the workers write it, and `LISTEN/NOTIFY` just tells readers "something changed, go look":

```sql
CREATE TABLE task_status (
    id            BIGSERIAL PRIMARY KEY,
    user_id       INT NOT NULL,
    envelope_id   TEXT NOT NULL,          -- logical task identity (see §2.2)
    run           INT NOT NULL,           -- which run of the logical task, from 1
    task_type     TEXT NOT NULL,          -- 'illumination' | 'spark' | 'search_index' | 'ingest'
    entity_type   TEXT NOT NULL,          -- 'capture' | 'spark'
    entity_id     INT NOT NULL,           -- the entity this task operates on
    status_code   INT NOT NULL,           -- integer discriminant of task::StatusCode
    attempts      INT NOT NULL DEFAULT 0, -- 1-based attempt number within this run
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (envelope_id, run)
);

-- The NOTIFY channel name (a constant in code, not a table)
-- SELECT pg_notify('task_status_channel', envelope_id) FROM ...;
```

> **Schema management:** the table is defined by the SeaORM model (`src/model/task_status.rs`) and created/synced automatically at startup via `conn.get_schema_registry("dreamscroll::model::*").sync(&conn)` in `database/postgres.rs`. There is no hand-written migration file yet.

> **Why `UNIQUE (envelope_id, run)` and not `UNIQUE (envelope_id)`:** one row per *run* is what makes reruns expressible. `envelope_id` alone identifies the logical task; adding `run` lets a rerun append a new row instead of overwriting the previous outcome. The pair is also the guard that makes duplicate submission safe — see §7.

- **Written by** `TaskMaster` (on enqueue via `submit_*`, and on transitions via `begin_attempt`/`finish_attempt`) — each write is followed by a `NOTIFY task_status_channel`.
- **Read by** `StatusListener` (the `LISTEN`/`NOTIFY` thread) and the SSE handler on connect (replay) and on every `NOTIFY`/poll tick (incremental), filtered by `user_id` + `task_types` + `entity_type`/`entity_id`.
- **`status_code`** stores the integer discriminant of `task::StatusCode` (`Queued=0`, `InProgress=1`, `Completed=2`, `ErrorWillRetry=3`, `ErrorExhausted=4`). The mapping lives in `task/status_code.rs`. **These integers are persisted — do not renumber them.**
- **`entity_type`/`entity_id`** are the queryable entity linkage. They are what makes "give me all the incomplete task statuses for capture 123" expressible (see §6.1).

> **Why a focused `task_status` table (not a generic `events` table):** task state is a first-class, non-trivial problem of its own — it has a lifecycle (`queued → in_progress → completed/error`), retries/attempts, and reruns. A dedicated table with typed columns (`task_type`, `envelope_id`, `status_code`, `attempts`) models that cleanly and is queryable. A generic `kind`+`payload` JSONB table would dilute this and make task-state queries awkward. **Capture lifecycle events are a separate, TBD concern** (see §13) and should get their own mechanism when we tackle them — not be shoehorned into `task_status`.

This also fixes a latent bug from the audit notes: *"No task retry/dead-letter in LocalTaskQueue — failed tasks silently dropped."* With a `task_status` table, `ErrorWillRetry`/`ErrorExhausted` states become observable.

### 6.1 The status vocabulary, the query API, and the retry policy


**The status vocabulary** (`task::StatusCode`, `src/task/status_code.rs`):

| Variant          | Code | Meaning                                                                 |
| ---------------- | ---- | ----------------------------------------------------------------------- |
| `Queued`         | 0    | Enqueued, not yet picked up.                                            |
| `InProgress`     | 1    | A worker is currently executing an attempt.                             |
| `Completed`      | 2    | Succeeded. **The only complete status.**                                |
| `ErrorWillRetry` | 3    | Failed, but the app still has retry budget — another attempt is coming. |
| `ErrorExhausted` | 4    | Failed permanently (budget spent, or the error was non-retryable).      |

> **`ErrorWillRetry`/`ErrorExhausted` are *computed outcomes*, not intrinsic properties of an error.** The same underlying failure is `ErrorWillRetry` on attempt 1 and `ErrorExhausted` on the final attempt. The decision is made by `AttemptOutcome::from_failure(err, attempt, max_attempts)`.

**The query API is deliberately "incomplete", not "non-terminal".** There are two entity-scoped entry points, both returning every row whose status is **not `Completed`** — i.e. `Queued`, `InProgress`, `ErrorWillRetry`, **and `ErrorExhausted`**:

- `TaskStatusTracker::query_incomplete_for_entity(user_id, entity_type, entity_id)` — the tasks for one entity (e.g. one capture).
- `TaskStatusTracker::query_incomplete_for_user(user_id)` — every outstanding task for a user, across all entities (e.g. a global "what's still running?" view).

Rationale:

- **`ErrorExhausted` is included on purpose.** The work never succeeded, so the user still wants to see it (and may want to retry it). It is *not* "done".
- **`Completed` is excluded on purpose.** Completed rows are subject to vacuuming over time, so an API that returned them would silently present an incomplete history. The API therefore cannot express "give me everything" — the usage pattern is enforced by what's available.
- The predicate is derived from `StatusCode::is_incomplete()` via `StatusCode::incomplete_codes()`, so the status set and the SQL predicate can never drift apart.

> **TODO(REVISIT) — index.** The primary read patterns are `WHERE user_id = ? AND entity_type = ? AND entity_id = ? AND status_code IN (...)` (entity-scoped) and `WHERE user_id = ? AND status_code IN (...)` (user-scoped), which currently only have the single-column `entity_id` index. A composite index on `(user_id, entity_type, entity_id, status_code)` is the right long-term shape for the former. **Deferred deliberately** — this is a single-user app and the table is tiny. Note SeaORM's derive only supports single-column `#[sea_orm(indexed)]` and composite `unique_key`, so a non-unique composite index needs raw SQL (e.g. `CREATE INDEX ... IF NOT EXISTS` alongside the schema sync in `database/postgres.rs`).

**The retry/exhaustion policy** (implemented 2026-09-15):

- **`Config.task_max_attempts`** (env `TASK_MAX_ATTEMPTS`, default `3`) is the app's own retry budget. It is threaded into `TaskMasterBuilder::max_attempts`.
- **`ApiError::is_retryable()`** classifies failures: 5xx (server errors) are transient and worth retrying; 4xx (client errors) are permanent — retrying identical input produces identical results.
- **`TaskMaster::begin_attempt(envelope)`** reads the persisted attempt count, increments it, writes `InProgress`, and returns the 1-based attempt number. Deriving the count from the DB (rather than Cloud Tasks' retry-count header) means it works identically for **every** backend, including `LocalTaskQueue`, which has no headers. It returns `None` when the task is already `Completed`, so an at-least-once redelivery of finished work is acked without resurrecting the row to `InProgress`.
- **`TaskMaster::finish_attempt(envelope, attempt, &result)`** writes the outcome and returns an `AttemptOutcome`.

**The key convention: the Cloud Tasks queue is always configured with MORE max retries than the app.** This means the app always exhausts its budget *first*, so it can ack the task and stop Cloud Tasks from spending its remaining retries. The HTTP mapping (`webhook::http_status_for_outcome`) follows from that:

| Outcome          | HTTP                        | Cloud Tasks behavior |
| ---------------- | --------------------------- | -------------------- |
| `Completed`      | `204 No Content`            | ack (stop)           |
| `ErrorExhausted` | `200 OK`                    | ack (stop)           |
| `ErrorWillRetry` | `500 Internal Server Error` | retry                |

> **Why `ErrorExhausted` returns 2xx:** Cloud Tasks retries on *any* non-2xx and stops on *any* 2xx — there is no "fail but don't retry" status code. Since the app's budget is smaller than the queue's, the app must ack to short-circuit the queue's remaining retries.

> **Why `ErrorWillRetry` uses `500`, not `503`:** Cloud Tasks treats `503` (and `429`) as *system* errors and responds by throttling the **whole queue's** dispatch rate. That is a queue-wide side effect we don't want from an ordinary per-task failure, so we use `500`, which retries the task without the congestion control.

> **The 204-vs-200 distinction is a debugging nicety, visible only in Cloud Run request logs.** Cloud Tasks' own `lastAttempt.responseStatus` is a `google.rpc.Status`, where every 2xx normalizes to `OK`, so it cannot distinguish the two acked cases. The `task_status` row remains the source of truth.

> **Local dev:** `LocalTaskQueue` does not retry at all (it logs and drops on handler error), so `config_local.env` sets `TASK_MAX_ATTEMPTS=1` — a local failure lands on `ErrorExhausted` immediately rather than appearing stuck at `ErrorWillRetry`.

### 6.2 `LISTEN/NOTIFY` mechanics and the connection budget

The one real constraint is **connection count**. The pool is capped at 5 (`max_connections(5)` in `database/postgres.rs`, sized for the `db-f1-micro` tier), and `LISTEN` requires a **dedicated, long-lived connection** (a pooled connection that returns to the pool would leak the `LISTEN` registration). So:

- **One dedicated `LISTEN` connection per instance** (not per SSE connection), owned by **`StatusListener`**. All SSE handlers on an instance share it via a small fan-out: `StatusListener` receives notifications and forwards them to in-process `tokio::sync::broadcast` *receivers* (one per SSE connection). This is fine — the in-process channel is now only a *local delivery* mechanism for notifications that already arrived via Postgres, not the source of truth. It cannot drift because it's just echoing DB notifications.
- **Budget check:** 1 dedicated `LISTEN` connection per instance + the normal pool of 5. With Cloud Run scaling to a handful of instances, this stays well within the `f1-micro` connection limit. If the tier is ever raised, this becomes even less of a concern.
- **Fallback:** if a dedicated `LISTEN` connection can't be established (or to be extra safe), the SSE handler falls back to a **poll tick** (re-query the DB every N seconds). This keeps correctness with zero extra connections — just slightly higher latency.

> **Why not one `LISTEN` connection per SSE connection?** That would multiply connections by concurrent users and blow the `f1-micro` budget. Sharing one `LISTEN` per instance and fanning out locally is the right trade-off: the DB is still the source of truth, and the local channel is a pure delivery optimization with no correctness role.

---

## 7. Runs and reruns ("Illuminate this again with a new model")

**Status: run dimension IMPLEMENTED 2026-09-16.** The `run` column, the
`(envelope_id, run)` unique constraint, submit-time refusal, and run-scoped
attempt counting are done. What remains deferred is the *rerun UX*: a
`model`/`force` field on `IlluminationTask`, a rerun endpoint, and relaxing the
idempotency guard in `logic/illuminate.rs` for an explicit rerun request.

### How runs work

`TaskEnvelope.envelope_id` names the **logical task** (`u1-illuminate-capture123`)
and `TaskEnvelope.run` names **one attempt to carry it out**, counting from 1.
`(envelope_id, run)` is unique and keys a `task_status` row, so a rerun appends
a row rather than overwriting the previous outcome.

A submission is planned by reading the latest run of the logical task
(`plan_submission`):

| Latest run                                         | Decision                                           |
| -------------------------------------------------- | -------------------------------------------------- |
| none                                               | start run 1                                        |
| in flight (`Queued`/`InProgress`/`ErrorWillRetry`) | **refuse** — the work is already queued or running |
| settled (`Completed`/`ErrorExhausted`)             | start `run + 1`                                    |

This gives two properties at once:

- **Duplicate submission is prevented.** Re-submitting work that is in flight is
  refused, so a double-clicked upload cannot queue the task twice.
- **Reruns work.** Once a run settles, the next submission starts a new run —
  so "illuminate this again" needs no special machinery, only a settled prior run.

The refusal is returned as `SubmitOutcome::RefusedInFlight { run }`, a normal
outcome rather than an `Err`: duplicates are expected, so callers should not log
them as failures. `SubmitOutcome::Enqueued { run }` reports the run that started.

> **Correctness rests on the unique index, not the read.** `plan_submission` is
> check-then-act: two concurrent submitters can both read "no prior run" and both
> try to insert run 1. The `(envelope_id, run)` unique index arbitrates — the
> loser's insert is a unique violation, which `create_run` maps to `Ok(false)`
> and `submit_inner` turns into `RefusedInFlight`. This is the same TOCTOU
> pattern tolerated elsewhere (`pragmatism.md`), except here the constraint makes
> it safe rather than merely unlikely.

Attempt counting is **per run**: each row carries its own `attempts`, so a rerun
starts at attempt 1.

> **`is_in_flight` is not `is_incomplete`.** They are different axes:
> `ErrorExhausted` is incomplete (the user should still see the failure) but not
> in flight (no worker will touch it again), which is exactly what makes it
> rerunnable. `StatusCode` exposes both predicates explicitly so they cannot be
> conflated.

### Queries return only the latest run

`query_incomplete_for_entity` / `query_incomplete_for_user` collapse to the
latest run per logical task — a rerun supersedes the run before it, and callers
want current state, not a run history.

The order matters: rows are collapsed to the latest run **before** the
incomplete predicate is applied. Filtering first would let an older incomplete
run shadow a newer completed one, reporting work as outstanding when it is done.
(That ordering bug is worth a test; see `completed_latest_run_hides_an_older_failed_run`.)

### What's still deferred (the rerun UX)

- A `model`/`force` field on `IlluminationTask` so a rerun can differ from the
  original (new model, new prompt).
- Relaxing the **idempotency guard in `logic/illuminate.rs`**
  (`if !capture.illuminations.is_empty() { skip }`), which silently swallows a
  rerun because the capture already has an illumination. Note this guard is *not*
  the dedupe mechanism — `LocalTaskQueue` does not dedupe, and submit-time
  refusal is now handled by the run logic above.
- A rerun endpoint + button. The handler calls
  `task_master.submit_illumination(user_id, IlluminationTask { .. })`; the run
  logic starts the next run automatically once the prior one has settled.

```html
<button hx-post="/detail/{{ capture.id }}/rerun"
        hx-vals='{"model": "gemini-2.5-pro"}'>
  Re-illuminate
</button>
```

The client's SSE listener sees the new run and shows "illuminating…" then
re-fetches when `Completed` arrives.

> **No way to reject a rerun after completion.** Because a settled run always
> permits a new one, a genuine duplicate submitted after completion is
> indistinguishable from an intentional rerun. Accepted: re-running is cheap and
> idempotent, so the distinction does not matter. Forcing a rerun *while one is
> in flight* would need an explicit `force` flag — deferred.

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

**Server side** — every task-status transition publishes an event onto the bus carrying `entity_type`/`entity_id` (plus `task_type`/`envelope_id`). For capture-scoped tasks `entity_type = "capture"` and `entity_id` **is** the capture id, which is the case that matters here. The SSE handler just forwards *all* of that user's events down the one connection. It does not care how many distinct entities are in flight:

```
5 uploads → 5 IlluminationTasks → 5× (Queued → InProgress → Completed|Error*) events
                                                          │
                                                          ▼
              one /events connection, N events, each tagged with entity_id
```

**Client side** — the key insight is that **htmx-ext-sse dispatches each SSE event as a DOM `CustomEvent` on the element that declares the listener**, and the event's `detail` carries the raw SSE `data`. So a card can listen for *its own* completion by filtering on the payload's `entity_id`.

### The two idiomatic client patterns

**Pattern A — one listener per card, filtered by `entity_id` (recommended for the timeline).** Each card carries a tiny listener that reacts only when the event's `entity_id` matches its own:

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

Because the server tags every event with `entity_id`, the card's `hx-trigger="sse:task-status"` fires for *every* task-status event — but the re-fetch is scoped to that card's own URL (`/cards/{{ capture.id }}`), so it only re-renders itself. The `data-capture-id` attribute is there for the optional JS status-pill path (below) to filter precisely.

> **Important subtlety:** `hx-trigger="sse:task-status"` fires on *every* task-status event, not just this card's. That's fine here because the re-fetch URL is per-card — a card re-fetching its own partial when *another* card changes is harmless (it just re-renders the same content). If you want to avoid even that, use the JS filter in Pattern B.

**Pattern B — a single JS listener that routes by `entity_id` (for precise fan-out / status pills).** One listener on the shared connection reads `event.detail`, checks `entity_id`, and updates only the matching card:

```js
// webui-v2.js — one listener, routes to the right card
document.body.addEventListener('sse:task-status', (e) => {
  const data = JSON.parse(e.detail.data);   // { task_type, envelope_id, entity_type, entity_id, status, attempts }
  if (data.entity_type !== 'capture') return;
  const card = document.querySelector(`#card-${data.entity_id}`);
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

### The one thing to get right: the event payload must carry `entity_id`

For the fan-out to work, every event must be self-describing. The `TaskStatusEvent` in §5.1a already denormalizes `entity_type`/`entity_id` for exactly this reason. For capture-scoped tasks `entity_id` **is** the capture id, so the capture-card fan-out works directly. If a task type ever has no single capture (e.g. a spark over many captures), the payload's `entity_type` tells the client which routing key is appropriate — the mechanism is identical.

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

1. **Worker (any instance)** writes the task-status row via `TaskMaster::begin_attempt`/`finish_attempt`, then `NOTIFY`s `task_status_channel`.
2. **Every instance** runs one dedicated `LISTEN` connection (owned by `StatusListener`). On a notification, it fans out locally to its connected SSE handlers, which re-query the DB for that user's matching rows and emit them.
3. **Replay on connect** reconciles any missed events — the SSE handler queries the DB for the user's matching task status on connect, so a browser that reconnects (or connects to a different instance) is instantly correct.
4. **Poll fallback** guarantees eventual correctness even if `LISTEN/NOTIFY` is unavailable — the SSE handler re-queries the DB on a modest interval.

There is **no in-process state that can drift or be lost** — the local channel is only a delivery optimization for notifications that already arrived from Postgres.

For Cloud Run specifically: SSE works fine through the Cloud Run ingress as long as the service **doesn't set a short request timeout** (SSE is a long-lived request). Cloud Run's default request timeout is 60s for the *first* byte, but a streaming response that keeps sending keep-alives is fine. Set the service timeout appropriately (e.g. 15 min) and rely on `KeepAlive::default()` to keep the connection alive. **The adaptive 5-min idle close (§8.6) must happen before the service timeout**, so the connection ends gracefully rather than being killed by Cloud Run.

---

## 10. File changes (summary — ✅ = done, ⬜ = pending)

| File                               | Change                                                                                                                                                                                               | Status |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| `src/task/task.rs`                 | `Task` trait (`task_type()`/`entity_type()`/`entity_id()`) + `TaskEnvelope<T>` (`user_id`, `envelope_id`, `run`, payload) + `TaskEnvelope::new(user_id, task, run)` / `make_envelope_id`             | ✅      |
| `src/task/taskqueue.rs`            | `TaskQueue<T>` trait, enqueue-only (takes `TaskEnvelope<T>`)                                                                                                                                         | ✅      |
| `src/task/taskqueue_local.rs`      | `LocalTaskQueue` — in-process mpsc + semaphore backend (no retry)                                                                                                                                    | ✅      |
| `src/task/taskqueue_cloudtask.rs`  | `CloudTaskQueue` — Google Cloud Tasks backend                                                                                                                                                        | ✅      |
| `src/task/taskqueue_pubsub.rs`     | **removed** (2026-09-15) — Pub/Sub support stripped out; Cloud Tasks is the focus                                                                                                                    | ✅      |
| `src/task/taskmaster.rs`           | `TaskMaster` — owns queues + `task_status`; `submit_*` / `begin_attempt` / `finish_attempt` / `query_*`; `update_status` is **private**; records `Queued` on enqueue; shared via `Arc`               | ✅      |
| `src/task/status_tracker.rs`       | `TaskStatusTracker` — owns all `task_status` persistence (create/update keyed by `(envelope_id, run)`); `latest_run`, `query_run_status`, `query_incomplete_for_entity`, `query_incomplete_for_user` | ✅      |
| `src/task/status_code.rs`          | `StatusCode` enum — `Queued`/`InProgress`/`Completed`/`ErrorWillRetry`/`ErrorExhausted`; `is_incomplete()`/`incomplete_codes()`; DB stores integer discriminant                                      | ✅      |
| `src/task/status_listener.rs`      | `StatusListener` — the `LISTEN`/`NOTIFY` thread (stub for now); one of two owners of `task_status`                                                                                                   | ⬜      |
| `src/task/beacon.rs`               | **removed** — replaced by `TaskMaster`                                                                                                                                                               | ✅      |
| `src/model/task_status.rs`         | `task_status` SeaORM model (`(envelope_id, run)` unique, `entity_type`/`entity_id`, `status_code`, `attempts`) — auto-synced at startup                                                              | ✅      |
| `src/api/apierror.rs`              | `ApiError::is_retryable()` — 5xx retryable, 4xx permanent                                                                                                                                            | ✅      |
| `src/config/schema.rs`             | `Config.task_max_attempts` (env `TASK_MAX_ATTEMPTS`, default 3)                                                                                                                                      | ✅      |
| `src/webhook/mod.rs`               | `http_status_for_outcome` — maps `AttemptOutcome` to the HTTP status Cloud Tasks sees                                                                                                                | ✅      |
| `src/webhook/webhook_state.rs`     | `WebhookState` carries `task_master: Arc<TaskMaster>`                                                                                                                                                | ✅      |
| `src/webhook/r_*.rs` (4 handlers)  | accept `TaskEnvelope<T>`; `begin_attempt`/`finish_attempt` around `logic::exec`; return `http_status_for_outcome`                                                                                    | ✅      |
| `src/events/mod.rs` *(new)*        | `TaskStatusEvent` struct + `StatusWriter` (write row + `NOTIFY`)                                                                                                                                     | ⬜      |
| `src/events/notifier.rs` *(new)*   | dedicated `LISTEN` connection + local fan-out to SSE receivers                                                                                                                                       | ⬜      |
| `src/webui/v2/maker.rs`            | add `/events` SSE route; thread `StatusListener` + notifier into `WebState`                                                                                                                          | ⬜      |
| `src/webui/v2/r_events.rs` *(new)* | SSE handler (replay from DB, listen for notifications, filter by user + task_types + entity ids, adaptive lifetime)                                                                                  | ⬜      |
| `src/webui/v2/r_rerun.rs` *(new)*  | rerun endpoint (deferred — see §7)                                                                                                                                                                   | ⬜      |
| `web/v2/templates/*.tera`          | add `hx-ext="sse"`, `sse-connect` (with `task_types`/`capture_ids`), `hx-trigger="sse:task-status"`; render per-status card state applied from SSE events (§5.5)                                     | ⬜      |
| `web/v2/static/webui-v2.js`        | extend upload notice to react to task-status events; reconnect `/events` on user interaction (adaptive lifetime)                                                                                     | ⬜      |

---

## 11. Why this is "simple, idiomatic, robust, flexible"

- **Simple:** The client is ~3 HTML attributes. The server uses Postgres `LISTEN/NOTIFY` (built into Postgres, zero new dependencies) + one small `StatusWriter`. No build step, no JS framework.
- **Idiomatic:** SSE is the canonical HTMX companion; `htmx-ext-sse` is the official extension. Axum has first-class SSE support. `LISTEN/NOTIFY` is the idiomatic Postgres pub/sub. `TaskMaster`/`StatusListener` are the two clean owners of task state.
- **Robust:** The `task_status` table is the **single canonical source of truth** — it survives reconnects, restarts, and multi-instance workers with no in-process state to drift. `LISTEN/NOTIFY` gives low latency; the poll fallback guarantees correctness; keep-alives + auto-reconnect handle flaky connections. The adaptive lifetime keeps idle connections from accumulating.
- **Flexible:** The `(task_type, envelope_id, entity_type, entity_id)` task-status model is generic — illumination, spark, search-index, ingest all flow through the same table/channel. Adding a new task type = implement `Task` (with its `entity_type`/`entity_id`), write a row + `NOTIFY` + add an `hx-trigger="sse:task-status"` line. Client subscription (`task_types` + `capture_ids`) keeps the stream relevant.

---

## 12. Implementation status

1. **`task_status` table + model** (foundation; also fixes the "no retry observability" audit note). ✅ *model done; auto-synced at startup*
2. **Task framework** — `Task` trait (with identity), `TaskEnvelope<T>`, `TaskQueue<T>` (Local + Cloud Tasks), `TaskMaster` (`submit_*`/`begin_attempt`/`finish_attempt`/`query_*`), `TaskStatusTracker`, `StatusCode`. ✅ *done*
3. **Status tracking end-to-end** — `submit_*` records `Queued`; webhook handlers record `InProgress` → `Completed`/`ErrorWillRetry`/`ErrorExhausted` via `begin_attempt`/`finish_attempt`. ✅ *done*
4. **Retry/exhaustion policy** — `task_max_attempts`, `ApiError::is_retryable()`, `AttemptOutcome`, and the `http_status_for_outcome` mapping (including the ack-on-exhaustion convention). ✅ *done (see §6.1)*
5. **Entity-scoped query API** — `query_incomplete_for_entity(user_id, entity_type, entity_id)` and `query_incomplete_for_user(user_id)`. ✅ *done (see §6.1)*
6. **`StatusWriter` + `NOTIFY`** — write row + `NOTIFY task_status_channel`. ⬜ *pending*
7. **`/events` SSE route** with user filtering + DB replay + poll fallback + subscription params (`task_types`, `capture_ids`). ⬜ *pending*
8. **Client wiring** (`hx-ext="sse"`, `sse-connect`, `hx-trigger="sse:task-status"`) — live updates for the *existing* upload flow should appear immediately. ⬜ *pending*
9. **Adaptive lifetime** (5-min idle close + reconnect-on-interaction) — the topology safeguard. ⬜ *pending*
10. **Rerun UX** (`model`/`force` on `IlluminationTask`, rerun endpoint, relax idempotency guard) — the run *dimension* itself is done (§7). ⬜ *deferred*

---

## 13. Open questions / follow-ups

- **SSE payload format:** thin JSON signals (recommended) vs. small HTML fragments for direct `sse-swap`. The design supports both; pick per use-case.
- **`task_status` retention:** add a cleanup/eviction policy to avoid unbounded table growth.
- **Two-owner rule (resolved):** only `TaskMaster` (writes) and `StatusListener` (reads for SSE) touch `task_status` directly. Everything else goes through `TaskMaster::submit_*`/`begin_attempt`/`finish_attempt`/`query_*`. This keeps the table's access surface tiny and auditable.
- **`TaskMaster.db` is optional (resolved):** when no DB is provided, `TaskMaster` runs enqueue-only (no `task_status` writes). This is used by util commands and tests that don't want background-task bookkeeping. `submit_*`/`begin_attempt`/`finish_attempt`/`query_*` no-op on the DB half in that mode.
- **Retry/attempts policy (RESOLVED 2026-09-15):** see §6.1. `attempts` is a 1-based attempt number derived from the persisted count; `task_max_attempts` (default 3) is the app's budget; `ApiError::is_retryable()` classifies failures; `ErrorWillRetry` vs `ErrorExhausted` is computed by `AttemptOutcome::from_failure`. The queue is always configured with more retries than the app, so the app exhausts first and acks.
- **Task identity (RESOLVED 2026-09-15; run-aware 2026-09-16):** identity lives on the `Task` trait (`task_type`/`entity_type`/`entity_id`) and `TaskEnvelope` carries a deterministic `envelope_id` (`u{user_id}-{task_type}-{entity_type}{entity_id}`) plus a 1-based `run`. No UUID, no `task_id.rs`. A re-submission targets the same logical task and either is refused (latest run in flight) or starts a new run — see §7.
- **`SparkTask.spark_id` is a placeholder (REVISIT):** `api/user/client.rs` mints a random `i32` (`uuid::Uuid::new_v4().as_u128() as i32`) because the real spark row id only exists after `insert_spark` runs at exec time. This makes spark's `envelope_id` non-deterministic. When spark gets a real identity (e.g. derived from its sorted `capture_ids`, or the planned "spark seed/spec" entity), it can join the deterministic scheme.
- **Spark is not queryable by capture (accepted for now):** a `SparkTask` operates on N captures but registers `entity_type = "spark"`, so a capture-scoped query will not surface it. **Accepted** — the plan is to introduce a "spark seed/spec" entity concept later, and the timeline will query task status by `entity_type = "spark"`. No N-entity join table is needed yet.
- **Single-row status lookups (RESOLVED 2026-09-15; run-scoped 2026-09-16):** the old full-row `query_status(envelope_id)` was removed — it had no callers. There are now two single-row reads, both used internally by the task framework: `latest_run(envelope_id)` (the decision input for submit, ordered by `run DESC`) and `query_run_status(envelope_id, run)` (used by `begin_attempt` to read a specific run's status and attempt count). Neither is user-scoped (`envelope_id` embeds `user_id`, so it is implicitly scoped, but the predicate is not enforced); the incomplete queries **are** explicitly scoped by `user_id`, since entity ids are not a security boundary.
- **`task_status` retention (REVISIT):** add a cleanup/eviction policy to avoid unbounded table growth. This is the reason the incomplete queries deliberately exclude `Completed` (see §6.1). *Tolerated — see `pragmatism.md`.*
- **Stuck tasks (REVISIT):** there is no heartbeat or timeout, so a task that is enqueued but never picked up (queue dropped, worker crash) stays `Queued` forever and looks active. Consider treating `Queued`/`InProgress` rows older than N minutes as dead, or a periodic sweep that stamps `ErrorExhausted`. *Tolerated — see `pragmatism.md`.*
- **Atomic upsert (REVISIT):** `TaskStatusTracker::record` is SELECT-then-INSERT/UPDATE. Now that `envelope_id` is `UNIQUE`, two concurrent writers could raise a unique-violation instead of silently duplicating. Consider a real `ON CONFLICT` upsert. *Tolerated — see `pragmatism.md`.*
- **Incomplete queries have no `ORDER BY` (REVISIT):** `query_incomplete_for_entity` and `query_incomplete_for_user` collapse to the latest run per logical task but return rows in non-deterministic order. Add an explicit order if the UI iterates the results. *Tolerated — see `pragmatism.md`.*
- **`submit_illumination` is the live capture path (RESOLVED 2026-09-16):** `IngestTask`/`logic/ingest.rs` were removed and the capture-create path now calls `submit_illumination` directly. `logic/illuminate::exec` runs illumination **and** search indexing as one unit of work. The `/_wh/cloudtask/illuminate` route is currently unused, reserved for future backfill / re-run flows.
- **`background` flag (REMOVED 2026-09-16):** the column was never populated (`record` hardcoded `false`) and the mandatory `capture_ids` subscription already prevents backfill noise, so it was removed rather than left as a speculative column. Backfill/bulk-task handling gets its own plan-and-branch session (see §4.3).
- **Admin backfill mis-attributes task status to the admin (REVISIT — deferred to the backfill session):** two related problems, both currently masked by the app being single-user:
  1. **Attribution.** `api/admin/backfill.rs` passes the requesting admin's `context.user_id()` to `submit_search_index`, so every backfill task's `task_status` row is owned by the **admin**, not the capture's owner. The task itself still works (`logic::search_index::exec` fetches via `service_api.get_captures`, which is not user-scoped), but the SSE layer filters by `user_id` for security — so the capture's owner would **not** see backfill status for their own captures. This is a correctness issue for SSE, not just a cosmetic inconsistency.
  2. **Candidate selection is global.** `api/service/need_search_index.rs::get_captures_need_search_index` has **no user filter** — it joins capture + illumination, filters `archived_at IS NULL`, and orders by `created_at DESC`. So `--all` enqueues tasks for *every* user's captures, all attributed to the admin.

  **Why it's deferred:** fixing attribution properly means `get_captures_need_search_index` must return `(capture_id, user_id)` pairs (not `Vec<i32>`) and `backfill::enqueue` must group by owner — real design work that belongs in the dedicated backfill plan-and-branch session (see §4.3). A half-fix would be worse than a documented gap. **Note:** `get_captures_need_search_index` also carries its own `TODO` — it returns recent captures without actually checking whether they need indexing, so candidate counts are inaccurate.
- **Envelope `user_id` is not validated against the capture owner (REVISIT):** `logic::spark::exec` derives `user_id` from the captures and never compares it to `envelope.user_id`, and `api/service/get_capture.rs` is explicitly **not** user-scoped. The webhook routes rely on Cloud Run OIDC, so this is defense-in-depth — but the envelope's `user_id` should be checked so status rows and data writes cannot diverge. *Tolerated — see `pragmatism.md`.*
- **Rerun support (partially done 2026-09-16):** the `run` column, `(envelope_id, run)` unique constraint, submit-time refusal for in-flight work, and run-scoped attempt counting are **implemented** (see §7). Still deferred: a `model`/`force` field on `IlluminationTask`, a rerun endpoint, and relaxing the idempotency guard in `logic/illuminate.rs`.
- **Cloud Run timeout:** confirm the service-level request timeout is set high enough for long-lived SSE connections (and above the 5-min adaptive idle close).
- **Multiple illuminations per capture:** decide whether reruns replace the existing illumination or append a new one (schema supports both; template currently renders `| first`).
- **Backfill / bulk tasks (deferred):** the `background` flag was removed (see §4.3). Backfill handling — marking tasks as bulk, surfacing backfill progress, an admin progress view, and fixing the `user_id` attribution + global candidate query above — will be tackled in a dedicated plan-and-branch session.
- **`capture_ids` is required for `task_status`:** the client always registers the captures it's tracking (see §4.2). This is simpler and more efficient than a "listen to all" default, and fits the app's bounded page size (a few hundred captures max). The client must keep its `capture_ids` list in sync with what's on screen (via the adaptive-lifetime reconnect, §8.6).
- **Reconnect-on-interaction cost:** confirm that re-issuing `sse-connect` on every HTMX request is cheap enough (it should be — SSE reconnect is lightweight and the DB replay reconciles state).
- **Initial-state source (resolved in §5.5):** for now, the `/events` "current status snapshot" is the canonical source of initial task status; the page-load HTML render does **not** include task status. Revisit later — having the page-load render also include status (so the initial view is correct before SSE connects / if SSE is unavailable) is a clean, additive change.
- **TBD — capture lifecycle events (created/deleted elsewhere):** explicitly out of scope for this phase. When we tackle it, it should get its **own mechanism** (likely a separate table/channel or a deliberate extension), not be shoehorned into `task_status`. The `capture_ids` subscription param and the single-SSE-connection-per-page design (§8.5) are forward-compatible with adding a second event type later.