# Real-time Task Status via SSE — Design

**Status:** **First end-to-end task-status SSE path implemented; still under
review.** TaskMaster now emits best-effort status notifications; the web app
starts one listener and fans events out to an authenticated `/events` route;
feed/detail pages subscribe and refresh the affected capture partial. Entity
availability has a wire type but no producer or client behavior yet.
**Scope:** Relay best-effort, low-latency **background-task status hints** to
HTMX clients over Server-Sent Events. This is informational UI feedback, not a
workflow engine, durable change log, or source of truth for task orchestration.

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
best-effort background-task status information to clients. It should:

- Scale across different **task types** (`illuminate`, `spark`, `search_index`).
- Let the client **subscribe to only what it cares about** (avoid noise, e.g.
  during a backfill).
- Handle **reruns** (e.g. "illuminate this again with a new model").
- **Respect the connection budget** of our narrow topology — long-lived SSE
  connections must not be held open indefinitely when idle.
- Minimize frontend cruft/complexity (the author is not a JS coder).

The user-facing purpose is deliberately narrow:

- tell the user our best-effort knowledge of what is happening in the backend;
- provide a smart hint that a small page component may be stale;
- let the client refresh or remove that component when appropriate.

Task status is **not** part of deterministic multi-stage pipeline logic. A task
can continue, finish, retry, or fail independently of whether a browser is
connected, and losing a notification is acceptable because the UI can remain
stale until its next refresh/reconnect. No durable transition history is
needed for this feature.

> **Scope boundary:** task-status publishing and delivery are the first
> integration target. The wire model also defines a generic `availability`
> event payload (`available`/`deleted`), intended for entity additions/removals
> that may affect a page. The type exists, but capture lifecycle publishing is
> not implemented. These events do not turn `task_run_status` into a catch-all
> event table.

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
  backfill, rerun handlers) can publish current status hints to.
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

> **Important distinction:** for this informational UI feature, the database
> row is the best available current status, while the SSE/`LISTEN` notification
> is a **best-effort hint**. A notification does not need to preserve or
> describe every transition. When the hint arrives, the SSE handler may use the
> included snapshot directly or re-query the current matching row before
> emitting a thin signal; the HTMX client then refreshes the relevant partial.
> A missed hint is acceptable. Replay/reconnect and optional polling merely
> improve the chance that the UI catches up; they are not a durable-log
> recovery protocol.

### 3.1 The update-event shape

Every update is a small, typed payload inside the generic `ServerEvent<E>`
envelope implemented in `src/sse/event.rs`:

```rust
pub struct ServerEvent<E> {
    pub schema_version: u8,
    pub event_type: ServerEventTypes,
    pub timestamp: DateTime<Utc>,
    pub entity_type: String,
    pub entity_id: i32,
    pub payload: E,
}

pub type TaskStatusEvent = ServerEvent<TaskStatusPayload>;
pub type AvailabilityEvent = ServerEvent<AvailabilityPayload>;
```

Task-status wire example:

```json
{ "schema_version": 1, "event_type": "task_status", "timestamp": "2026-09-21T18:42:10Z", "entity_type": "capture", "entity_id": 123, "payload": { "subchannel": "illuminate", "status": { "name": "complete_success", "discriminant": 4 }, "run": 1 } }
```

Availability wire example:

```json
{ "schema_version": 1, "event_type": "availability", "timestamp": "2026-09-21T18:42:15Z", "entity_type": "capture", "entity_id": 123, "payload": { "operation": "deleted" } }
```

This maps to the current state of a `task_run_status` row (see
`task-status.md` §3). It is not a historical event: the table stores one row
per logical task run and updates that row in place. The `TaskRunStatus` enum
lives in the task module; the DB stores only its integer discriminant.

**Design decision:** `TaskRunStatus` serializes as an object containing both its
stable snake-case name and persisted integer discriminant, for example
`{"name":"complete_success","discriminant":4}`. Deserialization validates
that the two representations agree. The persisted integer mapping remains
independent and must not change. A dedicated event DTO can still wrap the
status with task identity, but the status itself need not be duplicated as a
second string enum.

The generic envelope is a compile-time Rust convenience; `event_type` is the
runtime discriminator used by clients and other JSON consumers. The SSE event
name may remain specific (`task-status` or `availability`) for HTMX,
even though both payloads share the same envelope shape.

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
  `task_types=illuminate,spark`). Defaults to all.

```http
GET /events?task_types=illuminate,spark&capture_ids=123,456,789
```

The server filters on both `user_id` (always, for security) and the requested
`capture_ids`/`task_types` (for relevance). The client **re-registers** its
interests by reconnecting with new query params (see §6.3, which makes
re-registration natural).

> **How `capture_ids` maps to the DB:** the query param is a client-facing
> convenience. Internally it becomes `entity_type = 'capture' AND entity_id IN
> (...)`, matching the `task_run_status` columns. The SSE handler should use
> the batched latest-status snapshot query (`query_latest_status_for_entities`)
> for all registered capture IDs.

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

### 4.1 Best-known status and best-effort notifications

**An in-process-only `tokio::sync::broadcast` is insufficient across Cloud Run
instances.** The worker that completes a task can be a different instance than
the one holding the user's SSE connection, so an in-memory channel on instance
A would never see events published on instance B.

For task status, `task_run_status` is the persisted best-known current state.
It is not a durable event log, and SSE notifications are informational hints.
The listener's in-process broadcast is only per-instance fan-out to connected
SSE handlers, not a cross-instance source of truth.

The remaining question is purely about **latency**: how does a connected browser
learn about a new row *quickly* instead of waiting for a poll interval? Two
mechanisms, used together:

1. **Postgres `LISTEN`/`NOTIFY`** — the built-in database mechanism for
  cross-instance push. A worker writes the status row, then `NOTIFY`s a channel.
  Every instance's listener wakes up on the notification and fans it out to
  local SSE streams.
2. **A short poll fallback** — belt-and-suspenders. Even if `LISTEN/NOTIFY` is
   unavailable or a notification is missed, the SSE handler can re-query the DB
  on a modest interval (e.g. every 5–10s) to help the UI catch up. This is a
  product choice, not a strict correctness guarantee.

> **Why `LISTEN/NOTIFY` and not the in-process bus:** the in-process bus only
> works when producer and consumer share a process. In Cloud Run they don't.
> `LISTEN/NOTIFY` is the *distributed* equivalent — the same pub/sub idea, but
> the channel lives in Postgres, which every instance already shares.

> **SeaORM / SQLx boundary:** SeaORM remains responsible for ordinary
> `task_run_status` reads and writes. It can execute raw PostgreSQL statements,
> including `SELECT pg_notify(...)`, but it does not expose a first-class
> asynchronous notification listener comparable to SQLx's
> `sqlx::postgres::PgListener`. `LISTEN` requires a dedicated connection to
> remain checked out for the listener's lifetime while it waits for
> server-pushed messages. It cannot borrow a connection from the shared app
> pool: that connection would be pinned indefinitely, reducing pool capacity
> and coupling listener availability to ordinary query traffic. The standalone
> `src/sse` prototype therefore uses SQLx notification primitives over the shared SeaORM
> pool; it does not replace SeaORM as the application's ORM.
>
> The notifier shares SeaORM's underlying SQLx 0.9 `PgPool` via
> `DatabaseConnection::get_postgres_connection_pool()`; cloning `PgPool` clones
> the handle, not the pool or its connections. `pg_notify` borrows a pooled
> connection for one query, so no extra pool is needed. The `sqlx08` pool in
> `src/database/postgres.rs` exists only for `tower_sessions_sqlx_store` and is
> not used by SSE. In contrast, `ServerEventListener` opens and retains one
> dedicated connection because `LISTEN` must remain active while waiting.

### 4.2 Where task status gets written (and notified)

Task status is written at the natural choke points. Each status write publishes
a `TaskStatusEvent` after the row update; the notification is a **best-effort UI
hint**:

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

The implementation is split by responsibility:

- `src/sse/event.rs` defines the generic, versioned `ServerEvent<E>` envelope
  and typed task-status/availability payloads.
- `src/sse/notifier.rs` shares SeaORM's SQLx 0.9 pool, serializes a
  `ServerEvent<E>`, and sends it to `server_event_channel` using SQLx `pg_notify`.
- `src/sse/listener.rs` holds a dedicated SQLx `PgListener` connection, decodes
  notifications, and fans them out to per-instance SSE receivers.
- `src/webui/v2/r_events.rs` authenticates the connection, requires explicit
  `capture_ids`, sends current status snapshots, and forwards live events only
  for the connected user's registered captures.

The status row write and notification are separate operations. Notification
failure is logged and does not fail task processing; this is intentional for
best-effort UI feedback.

### 4.2.1 Notification payload: current-row snapshot

The channel semantic is deliberately simple: after a `task_run_status` row
changes, publish a typed update describing the new status. This is **best
effort**. The payload is useful for low-latency consumers, but is not a durable
event log; an initial snapshot on connection helps the UI catch up.

The standalone module now has a generic `ServerEvent<E>` envelope. The concrete
`TaskStatusEvent` uses a `TaskStatusPayload` containing the stable logical
`subchannel` (currently names such as `illuminate`), the logical `run` number,
and the compound-serialized `TaskRunStatus`. The envelope supplies the
timestamp and entity routing key. `AvailabilityEvent` uses the same envelope
with an `AvailabilityPayload`. These are informational update payloads;
additional metadata can be added later without changing the basic notification
semantics.

The trade-offs are acceptable for this table:

- **Payload size:** PostgreSQL notification payloads are limited to roughly 8 KB.
  The current row is comfortably below that, but fields should not be added to
  the snapshot casually. If the row grows beyond that limit, publish only a
  compact identity hint and re-query.
- **Staleness:** a later update can occur before a consumer processes an older
  notification. Consumers must treat the payload as a hint/snapshot and may
  re-read the row before emitting browser state.
- **Delivery:** notifications can be missed while a listener is disconnected,
  and they are not retained as an event log. Database replay and polling cover
  that gap.
- **Transaction boundary:** PostgreSQL delivers `NOTIFY` only when the
  transaction commits. The write and notification should therefore be issued
  in the same transaction when atomic row/payload correspondence matters.

### 4.3 The future SSE endpoint

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

### 4.4 Optional initial snapshot / reconnect behavior

SSE is **ephemeral** — a browser reconnect does not receive notifications sent
while it was disconnected. Since this feature is best-effort and informational,
the reconnect strategy can either send a fresh task-status snapshot from the DB
or simply resume listening for notifications. A snapshot improves UI catch-up
but does not constitute durable replay.

1. **Optional on connect**, the SSE handler can query `task_run_status` for this `user_id`
  matching the requested `entity_type`/`entity_id`/`task_types` and emits the
  latest row for each logical task immediately. The snapshot must include
  terminal states, especially `CompleteSuccess` and `CompleteFailure`, or the
  client cannot learn that work finished. The existing
  `TaskMaster::query_latest_status_for_entities` is the snapshot API and
  includes terminal as well as in-flight states. It batches all registered
  entity IDs in one query. Use it for initial UI snapshots; do not mistake it
  for a durable replay cursor.
2. **On every `NOTIFY` (or optional poll tick)**, re-query the DB for this user's matching
  current rows and emit refresh signals as needed. `NOTIFY` is not treated as
  an event-history cursor; the database snapshot reflects status when queried.

There is no requirement to recover every missed transition. The current route
does send one fresh snapshot for each subscribed capture when the stream starts;
this is a convenience for a more current UI, not durable event replay.

### 4.5 Where the initial (clean-slate) status comes from

**For now, the `/events` "current status snapshot" is the best available initial
task status.** The page-load HTML render does **not** include task
status — a deliberate simplification for this phase.

**The flow:**

1. **Page loads** → the HTML renders the captures (images, metadata) but **not**
   their task status. A capture that's mid-illumination simply shows no status
   pill yet.
2. **`/events` connects** → the handler's initial snapshot (§4.4) queries `task_run_status` for
  the registered `capture_ids` and emits the current statuses
   immediately. The client applies them (e.g. shows "illuminating…" on the
   matching cards).
3. **Subsequent updates** → SSE `task-status` events keep the status current as
   tasks transition.

**Why this is fine for now:**

- **It's simpler.** The page-load render doesn't need to join against
  `task_run_status` or render per-status states.
- **The snapshot is current when read.** The `/events` query reads the same
  `task_run_status` table used by task processing, reducing the stale window;
  it does not provide historical replay guarantees.
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

The main infrastructure constraint is **connection count**. The shared app
pool is capped at 5 (`max_connections(5)` in `database/postgres.rs`, sized for
the `db-f1-micro` tier). `LISTEN` requires a **dedicated, long-lived
connection**; it must not borrow from the app pool because it would pin a pooled
connection indefinitely and reduce capacity for regular queries. So:

- **One dedicated, long-lived `LISTEN` connection per instance** (not per SSE
  connection), owned by `ServerEventListener`. It is opened separately and is
  not acquired from or returned to the shared application `PgPool`.
  All SSE handlers on an instance share it via a small fan-out:
  `ServerEventListener` receives notifications and forwards them to in-process
  `tokio::sync::broadcast` *receivers* (one per SSE connection). This
  is fine — the in-process channel is now only a *local delivery* mechanism for
  notifications that already arrived via Postgres, not the source of truth. It
  cannot drift because it's just echoing DB notifications.
- **Budget check:** 1 dedicated `LISTEN` connection per instance + the shared
  app pool (capped at 5) and separate SQLx 0.8 session pool. With Cloud Run
  scaling to a handful of instances, this stays well
  within the `f1-micro` connection limit.
- **Fallback:** if a dedicated `LISTEN` connection can't be established, the
  SSE handler may use a **poll tick** (re-query the DB every N seconds) to help
  the UI catch up. This is optional and not a correctness guarantee.

> **Why not one `LISTEN` connection per SSE connection?** That would multiply
> connections by concurrent users and blow the `f1-micro` budget. Sharing one
> `LISTEN` per instance and fanning out locally is the right trade-off: the DB
> notification is a best-effort delivery hint, and local fan-out does not imply
> retained history or correctness guarantees.

---

## 5. Client-side design (minimal cruft)

### 5.1 One SSE connection per page

Add to the base templates (`index.html.tera`, `detail.html.tera`):

```html
<body hx-ext="sse">
  <div sse-connect="/events?task_types=illuminate,spark,search_index&capture_ids={{ visible_capture_ids }}"></div>

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
<div sse-connect="/events?task_types=illuminate&capture_ids={{ visible_capture_ids }}">
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
`LISTEN/NOTIFY` provides a cross-instance best-effort push hint:

1. **Worker (any instance)** writes the task-status row via
  `TaskMaster::submit_*`, `begin_attempt`, or `finish_attempt`, then publishes a
  typed `TaskStatusEvent` via `ServerEventNotifier` to `server_event_channel`.
2. **Every WebUI-enabled instance** runs one dedicated `LISTEN` connection
  (owned by `ServerEventListener`). On a notification, it fans the event out to
  local SSE handlers; each handler filters by owner and subscribed capture IDs.
3. **Optional initial snapshot** makes the UI more current on connect — the SSE
  handler may query the DB for matching status rows. This is not event replay,
  and it does not guarantee the browser observes every transition.
4. **Optional polling** may help the UI catch up if `LISTEN/NOTIFY` is
  unavailable; it is not a correctness guarantee.

The local channel is only a delivery optimization for notifications received
from Postgres; neither channel provides retained history.

For Cloud Run specifically: SSE works fine through the Cloud Run ingress as long
as the service **doesn't set a short request timeout** (SSE is a long-lived
request). A streaming response that keeps sending keep-alives is fine. Set the
service timeout appropriately (e.g. 15 min) and rely on `KeepAlive::default()`.
**The adaptive 5-min idle close (§5.5) must happen before the service timeout**,
so the connection ends gracefully rather than being killed by Cloud Run.

---

## 7. Why this is "simple, idiomatic, robust, flexible"

- **Simple:** The event schema and Postgres publisher/listener are small,
  separate modules. The client can use HTMX event triggers and one small JS
  router for precise entity fan-out. No JS framework is needed.
- **Idiomatic:** SSE is the canonical HTMX companion; `htmx-ext-sse` is the
  official extension. Axum has first-class SSE support. `LISTEN/NOTIFY` is the
  idiomatic Postgres pub/sub.
- **Robust:** `task_run_status` persists the best available current status across
  restarts. `LISTEN/NOTIFY` gives low-latency hints; optional initial snapshots
  and polling may help the UI catch up; keep-alives + auto-reconnect handle
  flaky connections. None of these components form a durable change log or
  guarantee delivery of every transition. The adaptive lifetime keeps idle
  connections from accumulating.
- **Flexible:** The `(task_type, envelope_id, entity_type, entity_id)` model is
  generic — illumination, spark, search-index all flow through the same
  table/channel. Adding a new task type = implement `Task` (with its
  `entity_type`/`entity_id`), write a row + `NOTIFY` + add an
  `hx-trigger="sse:task-status"` line.

---

## 8. Implementation status

**Implemented:** the typed, versioned event model; status notifications from
TaskMaster; one per-instance listener/fan-out; authenticated SSE route with
user and capture filtering plus initial status snapshots; and targeted capture
partial refresh in the client. **Not yet implemented:** adaptive idle lifetime,
entity-availability producers/consumers, and full-page status rendering.

| #   | Step                                                                        | Status |
| --- | --------------------------------------------------------------------------- | ------ |
| 1   | Integrate `ServerEventNotifier` with task-status writes                     | ✅      |
| 2   | Authenticated `/events` route — user/capture filtering and initial snapshot | ✅      |
| 3   | Client routing by entity ID and targeted capture partial refresh            | ✅      |
| 4   | Adaptive lifetime (5-min idle close + reconnect-on-interaction)             | ⬜      |

### File changes

| File                               | Change                                                                                      | Status |
| ---------------------------------- | ------------------------------------------------------------------------------------------- | ------ |
| `src/sse/event.rs`                 | Generic `ServerEvent<E>`, task-status and availability payloads, and wire serialization     | ✅      |
| `src/sse/notifier.rs`              | PostgreSQL `NOTIFY` publisher for typed server events                                       | ✅      |
| `src/sse/listener.rs`              | Dedicated PostgreSQL `LISTEN` receiver and event decoding                                   | ✅      |
| `src/bin/dreamscroll_web.rs`       | start WebUI listener and local fan-out                                                      | ✅      |
| `src/webui/v2/maker.rs`            | add `/events`, pass shared event receiver to `WebState`                                     | ✅      |
| `src/webui/v2/r_events.rs`         | authenticated stream, explicit capture subscriptions, batched initial snapshot, live filter | ✅      |
| `src/webui/v2/r_capture_card.rs`   | authenticated capture-card refresh endpoint                                                 | ✅      |
| `src/webui/v2/r_detail_partial.rs` | authenticated capture-detail partial refresh endpoint                                       | ✅      |
| `web/v2/templates/*.tera`          | stable capture IDs/data attributes for client event routing                                 | ✅      |
| `web/v2/static/webui-v2.js`        | subscribe, route by entity, refresh affected capture partial                                | ✅      |
| `Cargo.toml`                       | direct `futures-util` dependency for `stream::unfold` in the Axum SSE handler               | ✅      |

**Dependency note:** `futures-util` was already present transitively in
`Cargo.lock`; listing it directly in `Cargo.toml` makes the application's use
of `futures_util::stream::unfold` explicit. This did not introduce a new
transitive package. `async-stream` is also present transitively, but `unfold`
fits the stateful receiver loop directly and avoids adding another streaming
macro dependency.

---

## 9. Review findings — 2026-09-21

The following items were stale or inconsistent with the current code and must
be resolved as implementation work begins:

1. **Task type spelling:** the implementation returns `illuminate`, not
  `illumination`; all query examples and filters now use `illuminate`.
2. **Standalone event modules exist:** `src/sse/event.rs`, `notifier.rs`, and
  `listener.rs` define the event envelope and Postgres primitives. They remain
  disconnected from task persistence and the UI.
3. **Resolved:** TaskMaster now publishes a `TaskStatusEvent` after status row
  writes for queueing, submission failure, attempt start, and attempt outcome.
  Notification failure is logged but does not affect task processing.
4. **The status table is not an event log:** updates mutate one row identified
  by `(envelope_id, run)`. A notification payload must therefore identify the
  changed logical run (or be treated only as a wake-up hint); it cannot by
  itself represent every transition.
5. **Resolved:** the old `query_incomplete_for_entity` and
  `query_incomplete_for_user` names were misleading because those methods
  return the latest row for every task, including successful rows. They are now
  `query_latest_status_for_entity` and `query_latest_status_for_user`.
6. **Resolved:** `TaskRunStatus` now serializes directly as a compound object
  containing its stable snake-case name and integer discriminant. Deserialization
  rejects mismatched representations. The event envelope still carries identity
  and routing fields around that status.
7. **Queue availability is now enforced:** `TaskMaster` owns all three queues
  as required dependencies, and its production builder fails unless the
  illumination, search-index, and spark queues are supplied. The former
  missing-queue path, which returned `Enqueued` without a status row, has been
  removed. Unit tests use explicit test-only no-op queues where a test does not
  exercise that task type.
8. **Illumination is a pipeline:** `IlluminationTask` runs illumination and
  search indexing inside one worker attempt. The current status model exposes
  one aggregate `illuminate` task, not separate progress for the two stages.
  The first SSE version should document this as aggregate progress, or add a
  deliberate stage model; it should not imply stage-level feedback.
9. **Spark identity remains placeholder-based:** `SparkTask` uses a `spark_id`
  placeholder before the spark row exists and is not capture-queryable. SSE
  should treat spark subscriptions as spark-entity subscriptions until the
  planned spark identity work is done.
10. **Resolved:** the authenticated `/events` route and per-instance listener
  fan-out are wired. The stream requires explicit capture IDs, filters both by
  authenticated owner and entity membership, and begins with a latest-status
  snapshot before forwarding live events.
11. **Integrated prototype:** `src/sse` defines `ServerEvent<E>`, the
  `TaskStatusEvent` and `AvailabilityEvent` aliases, typed payloads, and
  versioned JSON serialization. `ServerEventNotifier` publishes to
  `server_event_channel`; `ServerEventListener` owns a dedicated
  `sqlx::postgres::PgListener` connection and decodes the two current event
  types. `webui-v2.js` routes task status by entity ID and refreshes only the
  affected capture card/detail partial.
12. **Product scope clarified:** task-status SSE is informational UI feedback
  only. It tells the user what the backend probably knows and hints that a slim
  page component may need refresh or removal. It is not deterministic pipeline
  logic, a workflow coordinator, or a durable change log. Missed notifications
  are acceptable.

## 10. Open questions / follow-ups

- **SSE payload format:** thin JSON signals remain the recommendation. The
  status field should use the directly serialized `TaskRunStatus`; small HTML
  fragments for direct `sse-swap` remain an optional future use-case.
- **Cloud Run timeout:** confirm the service-level request timeout is set high
  enough for long-lived SSE connections (and above the 5-min adaptive idle
  close).
- **Reconnect-on-interaction cost:** confirm that re-issuing `sse-connect` on
  every HTMX request is cheap enough (it should be — SSE reconnect is lightweight
  and the DB replay reconciles state).
- **Initial-state source (resolved in §4.5):** for now, an optional `/events`
  snapshot can provide best-available initial task status; the page-load HTML
  render does **not** include task status. Revisit later — having the page-load
  render also include status is a clean, additive change.
- **`capture_ids` is required for `task_run_status`:** the client always registers
  the captures it's tracking (§3.2). The client must keep its `capture_ids` list
  in sync with what's on screen (via the adaptive-lifetime reconnect, §5.5).
- **Backfill / bulk tasks (deferred):** the `background` flag was removed (§3.3).
  Backfill handling — marking tasks as bulk, surfacing backfill progress, an
  admin progress view, and fixing the `user_id` attribution + global candidate
  query — gets a dedicated plan-and-branch session. See `task-status.md` §8.
- **Capture lifecycle publishing:** the `AvailabilityEvent` wire type already
  models `available`/`deleted` operations for any entity type. Publishing these
  updates remains future work and should use a deliberate source/producer; do
  not shoehorn lifecycle events into `task_run_status`. The entity-scoped
  subscription and single-SSE-connection design are intended to accommodate
  this later.
