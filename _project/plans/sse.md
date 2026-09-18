# Real-time Task Status via SSE — Design

**Status:** **Not implemented.** This is a future, separate project. The task
framework it depends on is complete and merged — see `task-status.md`.
**Scope:** Relay accurate, up-to-date, low-latency **background-task status** to
HTMX clients over Server-Sent Events.

> **See also:**
> - `task-status.md` — the task framework this builds on (done).
> - `pragmatism.md` — the ledger of deliberately tolerated trade-offs.
> - `topology_and_throughput.md` — the connection/concurrency budgets that shape
>   the adaptive-lifetime design (§6.3).

---

## 1. Problem statement

When a user uploads a screenshot, the app enqueues an AI-powered illumination in
the background. There is **no notification to the client** when it completes —
the user must guess and manually refresh the page to see the updated
illumination data.

We want a **simple, idiomatic, robust, and flexible** strategy for relaying
accurate, up-to-date, low-latency background-task status to clients. It must:

- Scale across different **task types** (illumination, spark, search-index).
- Let the client **subscribe to only what it cares about** (avoid noise, e.g.
  during a backfill).
- Handle **reruns** (e.g. "illuminate this again with a new model").
- **Respect the connection budget** of our narrow topology — long-lived SSE
  connections must not be held open indefinitely when idle.
- Minimize frontend cruft/complexity (the author is not a JS coder).

> **Scope boundary:** this phase is about **task status only**. Capture lifecycle
> events (a capture uploaded/deleted on another device) are a **separate, TBD
> concern** — see §9. We deliberately do **not** generalize `task_run_status` into a
> catch-all event table.

---

## 2. Why SSE (and not WebSockets or polling)

| Option                                 | Pros                                                                                                                                                              | Cons                                                                               | Fit             |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | --------------- |
| **SSE**                                | Unidirectional push over plain HTTP; works through proxies/Cloud Run; auto-reconnect built into browsers; **htmx-ext-sse handles it declaratively with ~zero JS** | One-way (fine — you only need server→client)                                       | ✅ **Best fit**  |
| WebSockets                             | Bidirectional                                                                                                                                                     | Needs a stateful upgrade, more JS, more server cruft, awkward through some proxies | ❌ Overkill      |
| HTMX polling (`hx-trigger="every 2s"`) | Zero server work                                                                                                                                                  | Latency = poll interval; wasteful; still needs a "done?" endpoint                  | ⚠️ Fallback only |
| Long-polling                           | Simple                                                                                                                                                            | Reconnect churn, more complex server bookkeeping                                   | ❌               |

**SSE is the idiomatic HTMX answer.** The htmx team maintains `htmx-ext-sse`
specifically for this. The entire client-side surface is **three HTML
attributes** — no custom JS for the transport itself:

```html
<body hx-ext="sse">
  <div sse-connect="/events" sse-swap="task-status"></div>
</body>
```

That's it. The extension manages the `EventSource`, reconnects with exponential
backoff, and swaps in whatever HTML the server sends for a named event.

---

## 3. Core architectural idea: a **task event stream** + **thin signals**

The cleanest, most flexible design separates two concerns:

1. **A server-side task-status source** that any code (webhook logic, admin
   backfill, rerun handlers) can publish task-status transitions to.
2. **A thin SSE endpoint** that subscribes a user's browser to that source,
   filtered by `user_id` **and** by the client's requested task types / captures.

Crucially, **SSE should carry *thin signals*, not full HTML.** This is the
HATEOAS-friendly pattern that keeps the frontend cruft-free:

- SSE event: `{ task_type: "illuminate", envelope_id: "u1-illuminate-capture123", entity_type: "capture", entity_id: 123, status: "complete_success", attempts: 1 }`
- Client reacts with a normal HTMX request to re-fetch the *partial*
  (`/detail/{id}` fragment or `/cards`), which the existing Tera templates
  already render.

This means:

- **No HTML-over-SSE** (which would duplicate template logic and bloat the
  stream).
- **One generic mechanism** for *all* task types — no bespoke channel per
  feature.
- **Reruns "just work"** — the event is keyed by `(envelope_id, run)` and the
  client just re-fetches whatever partial is relevant.

### 3.1 The task-status event shape

Every task-status transition is a small, typed struct. The SSE event name is
constant (`event: task-status`); the payload carries the fields the client needs
to route and react:

```rust
// src/events/mod.rs — the shape of a task-status transition
pub struct TaskStatusEvent {
    pub task_type: String,   // "illumination" | "spark" | "search_index"
    pub envelope_id: String, // e.g. "u1-illuminate-capture123"
    pub entity_type: String, // "capture" | "spark"
    pub entity_id: i32,      // the entity this task operates on (fan-out key, §6.2)
    pub status: task::TaskRunStatus, // Queued | InProgress | ErrorWillRetry | CompleteSuccess | CompleteFailure
    pub attempts: i32,       // 1-based attempt number of the latest attempt
    pub user_id: i32,        // for per-user filtering
}
```

The SSE wire format is a flat JSON object:

```json
{ "task_type": "illuminate", "envelope_id": "u1-illuminate-capture123", "entity_type": "capture", "entity_id": 123, "status": "complete_success", "attempts": 1 }
```

This maps 1:1 onto a `task_run_status` row (see `task-status.md` §5). The
`TaskRunStatus` enum lives in the task module; the DB stores only its integer
discriminant.

> **Why a single named SSE event (`task-status`) rather than per-status names
> (`illumination-complete`, etc.):** named events are fine for a fixed set, but
> they don't scale to "subscribe to a subset" or "route by status" cleanly. A
> single event name with a `status` field in the payload keeps the client logic
> uniform and lets it filter on `status`/`task_type`/`entity_id` as needed.

### 3.2 Client subscription filtering

The client **explicitly registers the captures it cares about**. For
`task_run_status`, there is **no "listen to everything" default** — the client must
always send `capture_ids`. This is simpler, better, and more efficient:

1. **`capture_ids`** — **required** for `task_run_status`. The client lists the
   capture IDs it's currently rendering. Only events whose `entity_id` is in the
   list are delivered.
2. **`task_types`** — which task types to receive (e.g.
   `task_types=illumination,spark`). Defaults to all.

```http
GET /events?task_types=illumination,spark&capture_ids=123,456,789
```

The server filters on both `user_id` (always, for security) and the requested
`capture_ids`/`task_types` (for relevance). The client **re-registers** its
interests by reconnecting with new query params (see §6.3, which makes
re-registration natural).

> **How `capture_ids` maps to the DB:** the query param is a client-facing
> convenience. Internally it becomes `entity_type = 'capture' AND entity_id IN
> (...)`, matching the `task_run_status` columns. The SSE handler translates the
> subscription into the entity-scoped query (`query_incomplete_for_entity` per
> capture, or an `IN` variant).

> **Why force explicit `capture_ids` (rather than a "listen to all" default)?**
> - **It's simpler.** No special-casing of "all vs. some" — the rule is uniform:
>   *you get events for the captures you registered.*
> - **It's more efficient.** The server filters at the source, so the stream only
>   carries events the page can actually use.
> - **It fits the app's shape.** Because Dreamscroll is photo-heavy, a page
>   renders only a bounded number of captures — images are render/memory heavy,
>   so **a page will realistically never exceed a few hundred captures MAX**.
>   Registering a few hundred IDs is trivial, and far cheaper than streaming
>   every task event for the user.
> - **It makes backfill tracking natural.** If you're watching a specific set of
>   captures, you get their events. No separate "opt in to backfill" mode needed.
>
> **The trade-off:** the client must keep its `capture_ids` list in sync with
> what's on screen (as the user scrolls, add/remove IDs). This is a small amount
> of JS, and it's exactly the kind of bookkeeping the adaptive-lifetime reconnect
> (§6.3) already makes natural — each reconnect re-registers the current set.

### 3.3 Backfill / bulk tasks — deferred

An earlier revision carried a **`background` flag** on each task to distinguish
bulk/backfill work from user-initiated work. **That field was removed** — it was
never populated and backfill deserves its own design pass rather than a
speculative column.

**Why it isn't needed for this phase:** the mandatory `capture_ids` subscription
(§3.2) already prevents backfill noise. A backfill of hundreds of captures simply
never reaches a page unless that page is explicitly tracking one of those
captures. So the flag was never load-bearing as a filter.

> **Deferred:** backfill/bulk-task handling (marking tasks as background,
> surfacing backfill progress, an admin progress view) gets a dedicated
> plan-and-branch session. When it is, the natural shape is a new column on
> `task_run_status` plus a subscription param — but that decision is deliberately out
> of scope here.

---

## 4. Server-side design

### 4.1 The canonical source of truth is the DB — not an in-process bus

**The in-process `tokio::sync::broadcast` idea is dropped.** It's fragile in
Cloud Run: the worker that completes a task can be a *different instance* than
the one holding the user's SSE connection, so an in-memory channel on instance A
would never see events published on instance B. Any design that relies on
in-process state for correctness is wrong here.

**The `task_run_status` table (Postgres) is the single canonical source of truth.**
Every status transition is a row write. The SSE handler reads from the DB. There
is no separate in-memory event bus to keep in sync — the DB *is* the bus.

The remaining question is purely about **latency**: how does a connected browser
learn about a new row *quickly* instead of waiting for a poll interval? Two
mechanisms, used together:

1. **Postgres `LISTEN`/`NOTIFY`** — the idiomatic, dependency-free way to get
   cross-instance push. A worker writes the status row, then `NOTIFY`s a channel.
   Every instance's SSE handler holds a `LISTEN` connection and wakes up on the
   notification, then re-reads the row(s) from the DB. Near-real-time push across
   all instances with **zero new dependencies** and **no in-process state to
   drift**.
2. **A short poll fallback** — belt-and-suspenders. Even if `LISTEN/NOTIFY` is
   unavailable or a notification is missed, the SSE handler can re-query the DB
   on a modest interval (e.g. every 5–10s) to reconcile. This guarantees eventual
   correctness even in the worst case.

> **Why `LISTEN/NOTIFY` and not the in-process bus:** the in-process bus only
> works when producer and consumer share a process. In Cloud Run they don't.
> `LISTEN/NOTIFY` is the *distributed* equivalent — the same pub/sub idea, but
> the channel lives in Postgres, which every instance already shares.

### 4.2 Where task status gets written (and notified)

Task status is written at the natural choke points, each of which **writes a row
and `NOTIFY`s**:

1. **In `TaskMaster::submit_*`** — every task enqueue funnels through here. It
   records a `Queued` row on enqueue. This gives "queued" status for free,
   everywhere, including admin backfill.
2. **In the webhook handlers** (`webhook/r_illuminate.rs`, `r_spark.rs`,
   `r_search_index.rs`) — each handler deserializes a `TaskEnvelope<T>`, then
   calls `begin_attempt` (writes `InProgress` + the incremented attempt number)
  and `finish_attempt` (writes `CompleteSuccess`/`ErrorWillRetry`/`CompleteFailure` and
   returns the `AttemptOutcome` that decides the HTTP status) around the
   `logic/*::exec` call.

> **Note:** status is written in the **webhook handler**, not inside
> `logic/*::exec`. The `logic` functions stay pure (they take the bare task and
> don't know about task identity/status). The handler owns the envelope and
> reports status around the `exec` call.

> **Note:** `TaskMaster::update_status` is **private**. The raw setter is
> deliberately not exposed, so callers cannot write a status that disagrees with
> the attempt count or the retry decision.

A tiny helper encapsulates "write row + notify" so callers never touch the
channel directly:

```rust
// src/events/status_writer.rs
pub struct StatusWriter { /* holds a TaskMaster (or DB conn) + the notify channel name */ }

impl StatusWriter {
    pub async fn write(&self, event: &TaskStatusEvent) -> anyhow::Result<()> {
        // 1. UPSERT the task_run_status row (keyed by (envelope_id, run))
        // 2. NOTIFY task_status_channel, '<envelope_id>'  (payload is just a hint)
    }
}
```

> **Note:** `TaskMaster` already does the row write (step 1). `StatusWriter` is a
> thin wrapper that adds the `NOTIFY` (step 2) — or `TaskMaster` itself can own
> the notify. Either way the two-owner rule holds: only
> `TaskMaster`/`StatusListener` touch `task_run_status`.

### 4.3 The SSE endpoint

A new route in `webui/v2/maker.rs` (protected by the same `login_required` layer
as everything else — **auth is free**):

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
        // 1. Replay recent task status for this user matching the subscription (§4.4)
        // 2. Loop:
        //    a. Wait on the LISTEN channel (with a timeout) for a NOTIFY
        //    b. On notify (or timeout), re-query the DB for this user's matching rows
        //    c. Emit an Event for each
        // 3. Send a keep-alive comment periodically
        // 4. Enforce the adaptive lifetime (§6.3)
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()).into_response())
}
```

The stream emits `Event::default().event("task-status").json_data(&event)` — a
single named event whose payload carries the `status`/`task_type`/`entity_id`
fields.

**Filtering is layered:**

- **`user_id`** — always, for security. Never leak one user's task status to
  another.
- **`capture_ids`** — **required** for `task_run_status`. Only events for the
  registered capture IDs are delivered (§3.2).
- **`task_types`** — the client's requested task types.

Since the SSE connection is authenticated via the session cookie, `user_id` is
known at connect time and used both in the DB query and in the emitted events.

### 4.4 The "replay on connect" problem (now trivial)

SSE is **ephemeral** — if the browser reconnects (which htmx-ext-sse does
aggressively), it misses events that happened while disconnected. **Because the
DB is the source of truth, replay is just a query:**

1. **On connect**, the SSE handler queries `task_run_status` for this `user_id`
   matching the requested `entity_type`/`entity_id`/`task_types`, using the
  **incomplete** predicate (everything except `CompleteSuccess`), and emits those rows
   immediately. The client is instantly reconciled with reality — no missed
   events, no cross-instance gap.
2. **On every `NOTIFY` (or poll tick)**, re-query the DB for this user's matching
   rows that changed since the last emission, and emit them.

There is **no in-memory state to lose** and **no cross-instance coordination
problem** — the DB row is the single record, and both the producer (writer) and
the SSE handler (reader) agree on it. This is the entire point of making the DB
canonical.

### 4.5 Where the initial (clean-slate) status comes from

**For now, the `/events` "current status snapshot" is the canonical source of the
initial task status.** The page-load HTML render does **not** include task
status — a deliberate simplification for this phase.

**The flow:**

1. **Page loads** → the HTML renders the captures (images, metadata) but **not**
   their task status. A capture that's mid-illumination simply shows no status
   pill yet.
2. **`/events` connects** → the handler's replay (§4.4) queries `task_run_status` for
   the registered `capture_ids` and emits the current in-flight statuses
   immediately. The client applies them (e.g. shows "illuminating…" on the
   matching cards).
3. **Subsequent updates** → SSE `task-status` events keep the status current as
   tasks transition.

**Why this is fine for now:**

- **It's simpler.** The page-load render doesn't need to join against
  `task_run_status` or render per-status states.
- **The snapshot is authoritative.** Because the `/events` replay reads the same
  `task_run_status` table, the initial status is correct at connect time — no
  render→connect race, because the snapshot *is* the current state.
- **The gap is tiny.** The only window where a card shows no status is between
  page load and the SSE connection opening (sub-second).

> **Deferred (revisit later):** having the page-load HTML render also include task
> status (so the initial view is correct even before SSE connects, and works if
> SSE is unavailable). This is a clean, additive change later — the card template
> would render status from `task_run_status` at render time, and the `/events` replay
> would remain as the safety net.

> **Concretely:** a capture that's mid-illumination (10s) shows no status pill on
> initial page load; the `/events` replay delivers `{ status: "in_progress" }` for
> it on connect, the client shows "illuminating…"; when the task completes, the
> SSE `task-status` event fires, the client re-fetches the card partial, and it
> re-renders as "done."

### 4.6 `LISTEN/NOTIFY` mechanics and the connection budget

The one real constraint is **connection count**. The pool is capped at 5
(`max_connections(5)` in `database/postgres.rs`, sized for the `db-f1-micro`
tier), and `LISTEN` requires a **dedicated, long-lived connection** (a pooled
connection that returns to the pool would leak the `LISTEN` registration). So:

- **One dedicated `LISTEN` connection per instance** (not per SSE connection),
  owned by **`StatusListener`**. All SSE handlers on an instance share it via a
  small fan-out: `StatusListener` receives notifications and forwards them to
  in-process `tokio::sync::broadcast` *receivers* (one per SSE connection). This
  is fine — the in-process channel is now only a *local delivery* mechanism for
  notifications that already arrived via Postgres, not the source of truth. It
  cannot drift because it's just echoing DB notifications.
- **Budget check:** 1 dedicated `LISTEN` connection per instance + the normal
  pool of 5. With Cloud Run scaling to a handful of instances, this stays well
  within the `f1-micro` connection limit.
- **Fallback:** if a dedicated `LISTEN` connection can't be established (or to be
  extra safe), the SSE handler falls back to a **poll tick** (re-query the DB
  every N seconds). This keeps correctness with zero extra connections — just
  slightly higher latency.

> **Why not one `LISTEN` connection per SSE connection?** That would multiply
> connections by concurrent users and blow the `f1-micro` budget. Sharing one
> `LISTEN` per instance and fanning out locally is the right trade-off: the DB is
> still the source of truth, and the local channel is a pure delivery
> optimization with no correctness role.

---

## 5. Client-side design (minimal cruft)

### 5.1 One SSE connection per page

Add to the base templates (`index.html.tera`, `detail.html.tera`):

```html
<body hx-ext="sse">
  <div sse-connect="/events?task_types=illumination,spark,search_index&capture_ids={{ visible_capture_ids }}"></div>

  <!-- On a task-status event, re-fetch the detail partial -->
  <div hx-get="/detail/{{ capture.id }}"
       hx-trigger="sse:task-status"
       hx-target="#card-feed"
       hx-swap="innerHTML">
  </div>
</body>
```

**Subtlety:** `sse-swap` swaps the SSE *data* into the element, but we want to
*trigger a re-fetch* instead. The htmx-ext-sse docs give exactly the right tool:
**`hx-trigger="sse:<event>"`** for callbacks, and `sse-swap` for direct content
swap.

This is **pure HTML** — no JS. The server sends a single named event
`task-status` whose payload carries the `status`/`task_type`/`entity_id` fields;
htmx fires a GET to re-render the partial, and the existing Tera templates do the
rest. This is the HATEOAS pattern: **SSE says "something changed", HTMX fetches
the new state.**

> **Note on `capture_ids` in the URL:** `capture_ids` is **required** for
> `task_run_status` (§3.2). The template injects the page's visible capture IDs
> (`{{ visible_capture_ids }}`). Because the connection is re-established on
> reconnect (and on the adaptive-lifetime cycle in §6.3), the client naturally
> re-registers its interests each time.

### 5.2 A small status indicator (optional, still no JS)

Show a live "illuminating…" state with a second listener that swaps in a tiny
status partial:

```html
<div sse-connect="/events?task_types=illumination&capture_ids={{ visible_capture_ids }}">
  <div sse-swap="task-status">
    <span class="status-pill">queued</span>
  </div>
</div>
```

The server sends small HTML fragments for the relevant `task-status` payloads.
This keeps the "live status" feel without any imperative JS.

### 5.3 What about the upload flow?

The upload already uses a custom XHR with progress. After upload completes, the
server returns `{capture_id, detail_url}`. The client can **immediately open the
SSE connection scoped to that capture** (or just rely on the page-wide `/events`
connection) and show "Illuminating…" until the completion event arrives, then
re-fetch. Since the page already has `/events` connected, **zero new JS** — just
the existing `showUploadNotice` logic extended to also listen for the completion
event.

### 5.4 One SSE channel, many cards

**The question:** the timeline/home page can render 50+ capture cards, and a user
can upload 5 screenshots in quick succession so all 5 are queued/illuminating at
once. How does *one* page-level SSE channel relay the status of *all 5*?

**The answer: the SSE channel is a *multiplexed bus*, not a per-card
connection.** There is exactly **one** `EventSource` per page. It carries a
*stream of many named events*, each tagged with which capture it belongs to. The
client fans that single stream out to the right card. Nothing about the number of
cards or concurrent tasks changes the connection count — it's always 1.

**Server side** — every task-status transition publishes an event carrying
`entity_type`/`entity_id` (plus `task_type`/`envelope_id`). For capture-scoped
tasks `entity_type = "capture"` and `entity_id` **is** the capture id. The SSE
handler just forwards *all* of that user's events down the one connection:

```
5 uploads → 5 IlluminationTasks → 5× (Queued → InProgress → CompleteSuccess|CompleteFailure) events
                                                          │
                                                          ▼
              one /events connection, N events, each tagged with entity_id
```

**Client side** — htmx-ext-sse dispatches each SSE event as a DOM `CustomEvent`
on the element that declares the listener, and the event's `detail` carries the
raw SSE `data`. So a card can listen for *its own* completion by filtering on the
payload's `entity_id`.

**Pattern A — one listener per card, filtered by `entity_id` (recommended for the
timeline).** Each card carries a tiny listener that reacts only when the event's
`entity_id` matches its own:

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

Because the server tags every event with `entity_id`, the card's
`hx-trigger="sse:task-status"` fires for *every* task-status event — but the
re-fetch is scoped to that card's own URL, so it only re-renders itself.

> **Important subtlety:** `hx-trigger="sse:task-status"` fires on *every*
> task-status event, not just this card's. That's fine here because the re-fetch
> URL is per-card — a card re-fetching its own partial when *another* card
> changes is harmless (it just re-renders the same content). If you want to avoid
> even that, use the JS filter in Pattern B.

**Pattern B — a single JS listener that routes by `entity_id` (for precise
fan-out / status pills).** One listener on the shared connection reads
`event.detail`, checks `entity_id`, and updates only the matching card:

```js
// webui-v2.js — one listener, routes to the right card
document.body.addEventListener('sse:task-status', (e) => {
  const data = JSON.parse(e.detail.data);   // { task_type, envelope_id, entity_type, entity_id, status, attempts }
  if (data.entity_type !== 'capture') return;
  const card = document.querySelector(`#card-${data.entity_id}`);
  if (card) {
    card.classList.remove('is-illuminating');
    htmx.ajax('GET', `/cards/${data.entity_id}`, { target: card, swap: 'outerHTML' });
  }
});
```

This is the *precise* version: it touches only the card whose id matches, so 5
concurrent tasks update 5 distinct cards independently, in any completion order.

**Why this stays simple:**

- **Connection count is constant (1)** regardless of cards/tasks.
- **The server is dumb and generic** — it just forwards tagged events.
- **The client is either pure-HTML (Pattern A)** or a ~10-line JS router
  (Pattern B).
- **Ordering is naturally handled** — each event carries its own `entity_id`, so
  cards update independently and out-of-order completions are fine.

> **The one thing to get right: the event payload must carry `entity_id`.** For
> the fan-out to work, every event must be self-describing. `TaskStatusEvent`
> (§3.1) already denormalizes `entity_type`/`entity_id` for exactly this reason.
> If a task type ever has no single capture (e.g. a spark over many captures),
> the payload's `entity_type` tells the client which routing key is appropriate —
> the mechanism is identical.

### 5.5 Adaptive connection lifetime

**The problem:** our topology is narrow (see `topology_and_throughput.md`). Each
open SSE connection occupies a Cloud Run HTTP concurrency slot for its entire
duration, and on `db-f1-micro` the DB connection total is also tight. Holding
connections open indefinitely when idle is wasteful and risks exhausting the
budget as users accumulate.

**The strategy: a dynamic, adaptive lifetime.** By default the page listens to
`/events` for **5 minutes**, and the connection **automatically extends**
whenever:

- **(a) the user does something** (any interaction — a click, an HTMX request, an
  upload, a scroll-triggered fetch), or
- **(b) something meaningful happens on the server** (an event is delivered).

If neither happens for 5 minutes, the connection **closes gracefully** and the
page falls back to on-demand refresh (the pre-SSE behavior). The next user action
reopens it.

**Why this works:**

- **Idle pages don't hold connections forever.** A user who opens the timeline
  and walks away releases the slot after 5 min of inactivity.
- **Active pages stay live.** As long as the user is interacting or events are
  flowing, the connection keeps extending.
- **It's a natural fit for SSE.** SSE is designed to be re-established;
  htmx-ext-sse auto-reconnects. Closing after idle is just a graceful stream end,
  and the next interaction reconnects.

**How it's implemented:**

**Server side** — the SSE handler tracks two timestamps:

- `last_activity` — updated on every delivered event (condition b).
- `last_client_touch` — updated when the client signals activity (condition a).

The stream ends when `now - max(last_activity, last_client_touch) > IDLE_TIMEOUT`
(5 min). It sends a final event so the client knows the close was intentional,
not an error.

**Client side** — two mechanisms keep the connection alive while active:

1. **Server events extend it automatically** (condition b) — no client work.
2. **Client activity extends it** (condition a) — the client sends a lightweight
   signal on user interaction. The cleanest way: piggyback on the existing HTMX
   request cycle. Simpler still: since the page reconnects on the next action
   anyway, the client can just **reconnect** (re-issue `sse-connect`) on user
   activity rather than maintaining a heartbeat — the reconnect itself resets the
   5-min timer.

> **Recommended (simplest):** rely on **server events** to extend the lifetime
> during active work, and let the client **reconnect on user interaction**. No
> heartbeat endpoint needed. The flow:
> - Page loads → opens `/events` (5-min timer starts).
> - User interacts → htmx fires a request → on response, the client reconnects
>   `/events` (fresh 5-min timer). A few lines in `webui-v2.js` (listen for
>   `htmx:afterRequest` and re-issue the SSE connect).
> - Server event arrives → timer resets server-side.
> - 5 min of neither → server closes the stream; page is static until the next
>   interaction.

**Re-registration on reconnect.** Because the connection is re-established on
every reconnect, the client **re-registers its subscription each time** — the
`sse-connect` URL carries the current `task_types` and `capture_ids`. Since
`capture_ids` is **required** (§3.2), this re-registration is essential: as the
user scrolls and the set of visible cards changes, the page updates the
`sse-connect` URL to track only what's on screen. The adaptive lifetime and the
mandatory-`capture_ids` subscription model reinforce each other.

**Interaction with the topology budget:**

- **Idle connections are released** after 5 min → concurrency slots free up.
- **Active connections are bounded** to actual use → no unbounded accumulation.
- **Reconnect is cheap** and the DB replay (§4.4) reconciles any missed events.

> **Caveat:** the 5-min idle timeout must be **shorter than the Cloud Run request
> timeout** (default 5 min, max 60 min). If the service timeout is left at the
> 5-min default, an SSE connection idle for 5 min would be killed by Cloud Run
> anyway — so the adaptive close should happen *before* that, or the service
> timeout must be raised. Set the service timeout to e.g. 15 min and let the
> adaptive 5-min idle close happen first. (See `topology_and_throughput.md` §4.)

---

## 6. Deployment / multi-instance correctness

Because the worker can be a different instance than the browser's connection,
**the DB is the source of truth and `LISTEN/NOTIFY` is the cross-instance push**:

1. **Worker (any instance)** writes the task-status row via
   `TaskMaster::begin_attempt`/`finish_attempt`, then `NOTIFY`s
   `task_status_channel`.
2. **Every instance** runs one dedicated `LISTEN` connection (owned by
   `StatusListener`). On a notification, it fans out locally to its connected SSE
   handlers, which re-query the DB for that user's matching rows and emit them.
3. **Replay on connect** reconciles any missed events — the SSE handler queries
   the DB for the user's matching task status on connect, so a browser that
   reconnects (or connects to a different instance) is instantly correct.
4. **Poll fallback** guarantees eventual correctness even if `LISTEN/NOTIFY` is
   unavailable.

There is **no in-process state that can drift or be lost** — the local channel is
only a delivery optimization for notifications that already arrived from
Postgres.

For Cloud Run specifically: SSE works fine through the Cloud Run ingress as long
as the service **doesn't set a short request timeout** (SSE is a long-lived
request). A streaming response that keeps sending keep-alives is fine. Set the
service timeout appropriately (e.g. 15 min) and rely on `KeepAlive::default()`.
**The adaptive 5-min idle close (§5.5) must happen before the service timeout**,
so the connection ends gracefully rather than being killed by Cloud Run.

---

## 7. Why this is "simple, idiomatic, robust, flexible"

- **Simple:** The client is ~3 HTML attributes. The server uses Postgres
  `LISTEN/NOTIFY` (built into Postgres, zero new dependencies) + one small
  `StatusWriter`. No build step, no JS framework.
- **Idiomatic:** SSE is the canonical HTMX companion; `htmx-ext-sse` is the
  official extension. Axum has first-class SSE support. `LISTEN/NOTIFY` is the
  idiomatic Postgres pub/sub.
- **Robust:** The `task_run_status` table is the **single canonical source of
  truth** — it survives reconnects, restarts, and multi-instance workers with no
  in-process state to drift. `LISTEN/NOTIFY` gives low latency; the poll fallback
  guarantees correctness; keep-alives + auto-reconnect handle flaky connections.
  The adaptive lifetime keeps idle connections from accumulating.
- **Flexible:** The `(task_type, envelope_id, entity_type, entity_id)` model is
  generic — illumination, spark, search-index all flow through the same
  table/channel. Adding a new task type = implement `Task` (with its
  `entity_type`/`entity_id`), write a row + `NOTIFY` + add an
  `hx-trigger="sse:task-status"` line.

---

## 8. Implementation status

**Nothing in this document is implemented yet.** The task framework it depends on
is complete (`task-status.md`).

| #   | Step                                                                                          | Status |
| --- | --------------------------------------------------------------------------------------------- | ------ |
| 1   | `StatusWriter` + `NOTIFY task_status_channel`                                                 | ⬜      |
| 2   | `/events` SSE route — user filtering + DB replay + poll fallback + `task_types`/`capture_ids` | ⬜      |
| 3   | Client wiring (`hx-ext="sse"`, `sse-connect`, `hx-trigger="sse:task-status"`)                 | ⬜      |
| 4   | Adaptive lifetime (5-min idle close + reconnect-on-interaction)                               | ⬜      |

### File changes

| File                               | Change                                                                                                              | Status |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------ |
| `src/events/mod.rs` *(new)*        | `TaskStatusEvent` struct + `StatusWriter` (write row + `NOTIFY`)                                                    | ⬜      |
| `src/events/notifier.rs` *(new)*   | dedicated `LISTEN` connection + local fan-out to SSE receivers                                                      | ⬜      |
| `src/task/status_listener.rs`      | `StatusListener` — the `LISTEN`/`NOTIFY` thread (stub exists today)                                                 | ⬜      |
| `src/webui/v2/maker.rs`            | add `/events` SSE route; thread `StatusListener` into `WebState`                                                    | ⬜      |
| `src/webui/v2/r_events.rs` *(new)* | SSE handler (replay from DB, listen for notifications, filter by user + task_types + entity ids, adaptive lifetime) | ⬜      |
| `web/v2/templates/*.tera`          | add `hx-ext="sse"`, `sse-connect` (with `task_types`/`capture_ids`), `hx-trigger="sse:task-status"`                 | ⬜      |
| `web/v2/static/webui-v2.js`        | extend upload notice to react to task-status events; reconnect `/events` on user interaction (adaptive lifetime)    | ⬜      |

---

## 9. Open questions / follow-ups

- **SSE payload format:** thin JSON signals (recommended) vs. small HTML
  fragments for direct `sse-swap`. The design supports both; pick per use-case.
- **Cloud Run timeout:** confirm the service-level request timeout is set high
  enough for long-lived SSE connections (and above the 5-min adaptive idle
  close).
- **Reconnect-on-interaction cost:** confirm that re-issuing `sse-connect` on
  every HTMX request is cheap enough (it should be — SSE reconnect is lightweight
  and the DB replay reconciles state).
- **Initial-state source (resolved in §4.5):** for now, the `/events` snapshot is
  the canonical source of initial task status; the page-load HTML render does
  **not** include task status. Revisit later — having the page-load render also
  include status is a clean, additive change.
- **`capture_ids` is required for `task_run_status`:** the client always registers
  the captures it's tracking (§3.2). The client must keep its `capture_ids` list
  in sync with what's on screen (via the adaptive-lifetime reconnect, §5.5).
- **Backfill / bulk tasks (deferred):** the `background` flag was removed (§3.3).
  Backfill handling — marking tasks as bulk, surfacing backfill progress, an
  admin progress view, and fixing the `user_id` attribution + global candidate
  query — gets a dedicated plan-and-branch session. See `task-status.md` §8.
- **TBD — capture lifecycle events (created/deleted elsewhere):** explicitly out
  of scope for this phase. When we tackle it, it should get its **own mechanism**
  (likely a separate table/channel or a deliberate extension), not be shoehorned
  into `task_run_status`. The `capture_ids` subscription param and the
  single-SSE-connection-per-page design (§5.4) are forward-compatible with adding
  a second event type later.
