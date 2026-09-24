# Real-time Task Status via SSE — Design

**Status:** Task-status SSE is wired as one authenticated stream per page.
`capture_ids` selects a one-time initial status snapshot in the SSE response;
after that snapshot, the same connection receives all live task-status events
for the authenticated user. Entity availability has a wire type but no producer
or client behavior yet.
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
- Catch up the page's initially rendered captures, then keep the protocol
  simple by forwarding all live status events for the authenticated user.
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

  restarts. `LISTEN/NOTIFY` gives low-latency hints; normal page refresh can
  help the UI catch up; keep-alives + native EventSource reconnect handle
  flaky connections. This is not a durable change
---

## 2. Why SSE (and not WebSockets or polling)

| Option                                 | Pros                                                                                                                                                              | Cons                                                                               | Fit             |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | --------------- |
| **SSE**                                | Unidirectional push over plain HTTP; works through proxies/Cloud Run; auto-reconnect built into browsers; **htmx-ext-sse handles it declaratively with ~zero JS** | One-way (fine — you only need server→client)                                       | ✅ **Best fit**  |
| WebSockets                             | Bidirectional                                                                                                                                                     | Needs a stateful upgrade, more JS, more server cruft, awkward through some proxies | ❌ Overkill      |
| HTMX polling (`hx-trigger="every 2s"`) | Zero server work                                                                                                                                                  | Latency = poll interval; wasteful; still needs a "done?" endpoint                  | ⚠️ Fallback only |
| Long-polling                           | Simple                                                                                                                                                            | Reconnect churn, more complex server bookkeeping                                   | ❌               |

**SSE is a natural fit beside HTMX.** The htmx team maintains `htmx-ext-sse`
for declarative integrations, but this project currently uses native browser
`EventSource` plus a small JS router because events need routing by entity ID.
The earlier three-attribute example below is conceptual, not current wiring:

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
> describe every transition. The SSE handler forwards the received typed hint;
> the HTMX client then refreshes the relevant partial.
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

**Subscription decision:** use one stable, user-authenticated `/events` stream
per page. `capture_ids` is used only for the initial catch-up snapshot emitted
at the beginning of the SSE response. After the snapshot, the server sends all
live task-status events for that user without entity filtering. The browser's
JS router checks `entity_type`/`entity_id` and only refreshes entities currently
present in the DOM. This avoids a registration flow and connection churn as
cards enter or leave the feed.

This means the browser does **not** subscribe/unsubscribe as elements enter or
leave the page. It opens one stream when the page loads and keeps that URL for
the page lifetime. Native EventSource may reconnect after network/server
failure; application code should not close/recreate it on HTMX swaps or ordinary
user interaction.

The stream lifecycle is simple: subscribe to the local event receiver first,
query latest status for the requested capture IDs, emit those rows as ordinary
`task-status` events, then continue consuming the already-subscribed live
receiver. Subscribing first avoids a gap while the snapshot query runs; a
duplicate hint around the handoff is harmless because events trigger a refresh
of current state.

This trades narrower live filtering for a stable connection:

- Every live task-status hint for the user may reach the browser, including
  hints for entities not currently rendered (e.g. backfill). The client cheaply
  ignores those IDs. Backfill-specific suppression is deferred to a separate
  plan and feature branch.
- User filtering remains server-side and mandatory. Client-side DOM routing is
  only a relevance optimization: it must never be used as an authorization
  boundary. Follow-up partial requests remain protected and user-scoped.
- Future entity-availability hints can share the same stable stream and be
  routed by entity ID/type in the client.

Native `EventSource` is a GET-only stream with no subscription-update message.
Recreating it is technically valid and `close()` prevents intentional overlap,
but it creates extra `/events` requests and snapshot/query work. If server-side
dynamic interests become necessary later, design an explicit subscription
update protocol separately rather than reconnecting on every DOM change.

```http
GET /events?capture_ids=12,34,56
```

The route scopes both the initial snapshot query and live notifications to the
authenticated `user_id`. Snapshot entity IDs are only a catch-up selection;
they do not filter subsequent notifications. The client determines whether a
live entity is currently relevant by looking for its DOM target. A task update
for a non-rendered entity is ignored without causing a partial request.

The initial catch-up snapshot emits at most one refresh hint per entity, even
when multiple task types have status rows for that entity. This is only
snapshot coalescing; subsequent live task-status updates are still forwarded
individually.

The browser only fetches a partial for `ErrorWillRetry`, `CompleteSuccess`, or
`CompleteFailure`. It ignores `Queued`, `InProgress`, and `SubmissionFailed`,
which do not by themselves indicate newly available capture content. The same
filter applies to catch-up and live hints. `ErrorWillRetry` and
`CompleteFailure` remain refresh-worthy because a task may have written useful
content before a later pipeline step failed.

### 3.3 Backfill / bulk tasks — deferred
**Why it isn't needed for this phase:** the SSE route filters by user and
the client only refreshes entities present in the DOM. Off-screen/backfill
hints may reach the browser but are ignored; the flag is not needed for UI
correctness.

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

Postgres `LISTEN`/`NOTIFY` carries cross-instance best-effort hints. A worker
writes the status row, then notifies the shared channel. Each WebUI-enabled
instance's listener forwards received events to its local SSE streams. No poll
fallback is currently implemented.

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

Task status is written at the natural lifecycle choke points. `TaskMaster`
chooses statuses and coordinates queue ordering; its private `TaskRunTracker`
owns row persistence and publishes a **best-effort UI hint** after each
successful status write:

1. **During `TaskMaster::submit_*`** — each submission creates the `Queued` row
  and its event before enqueueing. A definite enqueue failure updates it to
  `SubmissionFailed` and publishes that result. A duplicate insert emits no
  event.
2. **In the webhook handlers** (`webhook/r_illuminate.rs`, `r_spark.rs`,
   `r_search_index.rs`) — each handler deserializes a `TaskEnvelope<T>`, then
  calls `begin_attempt` (which writes `InProgress` with the incremented attempt
  number) and `finish_attempt` (which writes `CompleteSuccess`, `ErrorWillRetry`,
  or `CompleteFailure` and returns the status used to decide the HTTP response)
  around the `logic/*::exec` call. Tracker events follow successful row updates.

> **Note:** status is written in the **webhook handler**, not inside
> `logic/*::exec`. The `logic` functions stay pure (they take the bare task and
> don't know about task identity/status). The handler owns the envelope and
> reports status around the `exec` call.

> **Note:** `TaskRunTracker` is private to the `task` module and owned by
> `TaskMaster`. It performs persistence and best-effort notification together;
> TaskMaster remains responsible for lifecycle policy, attempts, retry decisions,
> and ordering with queue operations.

The implementation is split by responsibility:

- `src/sse/event.rs` defines the generic, versioned `ServerEvent<E>` envelope
  and typed task-status/availability payloads.
- `src/sse/notifier.rs` shares SeaORM's SQLx 0.9 pool, serializes a
  `ServerEvent<E>`, and sends it to `server_event_channel` using SQLx `pg_notify`.
- `TaskMaster` is the public task lifecycle boundary and coordinates lifecycle
  policy with queue operations. Its private `TaskRunTracker` dependency owns
  status persistence and invokes an injected `ServerEventNotifier` after
  successful writes. `task::make_task_master` selects the PostgreSQL notifier
  and injects it through `TaskMasterBuilder`; notification errors remain best
  effort and do not fail persistence.
- `src/sse/listener.rs` holds a dedicated SQLx `PgListener` connection, decodes
  notifications, and fans them out to per-instance SSE receivers.
- `src/webui/v2/r_events.rs` authenticates the connection, emits a one-time
  latest-status snapshot for the requested capture IDs, then forwards every
  live task-status event for that user.

The status row write and notification are separate operations within the
tracker. Notification failure is logged and does not fail task processing;
this is intentional for best-effort UI feedback.

### 4.2.1 Notification payload: current-row snapshot

The channel semantic is deliberately simple: after a `task_run_status` row
changes, publish a typed update describing the new status. This is **best
effort**. The payload is useful for low-latency consumers, but is not a durable
event log. The stable-stream design does not query/send an initial snapshot.

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
  and they are not retained as an event log. The connect-time current-state
  snapshot helps with status already present for the page's initial captures;
  it does not replay missed transitions or cover arbitrary disconnected time.
- **Transaction boundary:** PostgreSQL delivers `NOTIFY` only when the
  transaction commits. The write and notification should therefore be issued
  in the same transaction when atomic row/payload correspondence matters.

### 4.3 The SSE endpoint

A new route in `webui/v2/maker.rs` (protected by the same `login_required` layer
as everything else — **auth is free**):

```rust
// src/webui/v2/r_events.rs
pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let user_id = user.id;

    let stream = /* await authenticated user's task-status hints */;
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()).into_response())
}
```

The stream emits `Event::default().event("task-status").json_data(&event)` — a
single named event whose payload carries the `status`/`task_type`/`entity_id`
fields.

**Filtering is layered:**

- **`user_id`** — always, for security. Never leak one user's task status to
  another.
- **Entity membership** — not filtered on the server. The client routes by
  `entity_type`/`entity_id` and only refreshes a matching rendered component.

Since the SSE connection is authenticated via the session cookie, `user_id` is
known at connect time and used to filter received notifications. Entity
relevance is intentionally decided in the browser, not by resubscribing the
server whenever visible cards change.

### 4.4 Delivery and reconnect semantics

SSE is **ephemeral** — a browser reconnect receives a fresh snapshot for the
capture IDs in its URL, but not a replay of missed transitions. The browser
client closes a failed native `EventSource` and creates a replacement using
capped exponential backoff with jitter (starting near one second and capping
near one minute). A successful `open` resets the backoff. This avoids native
EventSource's short fixed retry loop generating repeated `/events` requests
while the local server is stopped. Independently, the client closes the stream
after five minutes without user interaction and while the tab is hidden; user
activity or tab visibility reconnects it, and the initial snapshot catches up
current rendered captures. The server caps each response at four minutes so
streams periodically end even if a tab remains continuously active. The initial
snapshot is current state, not a transition log; updates remain informational
hints.

Activity does not bypass a pending failure backoff: while the server is
unavailable, scroll/pointer events only refresh the idle clock and do not start
new connection attempts. A single reconnect timer gates attempts until the
scheduled backoff expires.

Feed swaps update the DOM and the JS router's possible refresh targets; they do
not change or reopen the EventSource subscription. The catch-up ID set is fixed
when the page first opens the stream. Newly displayed entities still receive
future live hints; statuses already current before they became visible are
obtained through normal page rendering/refresh.

### 4.5 Initial status catch-up

The same `/events` response begins with the latest task status rows for the
page's initial capture IDs, then continues as a live user-wide stream. Page-load
HTML does not need to join task status.

**The flow:**

1. **Page loads** → the HTML renders the captures (images, metadata) but **not**
   their task status. A capture that's mid-illumination simply shows no status
   pill yet.
2. **`/events` connects** → the handler subscribes to live notifications, then
  emits the latest status for the capture IDs in the URL as `task-status`
  events on this response.
3. **Subsequent updates** → the same response streams all live task-status
  events for the authenticated user. The client refreshes matching rendered
  entities. A race may cause a duplicate event; partial refreshes are
  idempotent enough for this best-effort UI use.

**Why this is fine for now:**

- **It's simpler.** The page-load render doesn't need to join against
  `task_run_status` or render per-status states.
- A stable stream avoids per-card requests and feed-change reconnections.
- The catch-up is one batched query, scoped by user and selected entity IDs.

> **Deferred (revisit later):** having the page-load HTML render include task
> status, so it is visible even before SSE connects and when SSE is unavailable.

> **Concretely:** the catch-up reflects the latest task row for each requested
> logical task. It does not replay every transition or guarantee that a missed
> notification was observed.

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

**Shutdown behavior:** `dreamscroll_web` has a single watch-based shutdown
signal shared with the listener task and each SSE response stream. Ctrl-C on
macOS and SIGTERM on Cloud Run first trigger Axum graceful shutdown, then signal
these long-lived SSE tasks to exit. Without this propagation, open SSE streams
can keep graceful shutdown waiting indefinitely.

---

## 5. Client-side design (minimal cruft)

### 5.1 One stable SSE connection per page

Create one native `EventSource('/events')` per page and keep it open for that
page's lifetime (subject to normal browser/network reconnects and process
shutdown). The initial page capture IDs are included in the URL for the
one-time catch-up snapshot. The client does not update this list or recreate the
EventSource after feed swaps.

The route authenticates the session and filters notifications by `user_id`.
The URL carries no capture list. Each `task-status` payload contains
`entity_type`/`entity_id`; `webui-v2.js` checks whether the matching card is
currently in the DOM, then asks HTMX to fetch and replace just that partial.
This keeps one stream while retaining precise per-card refreshes.

**Why not resubscribe on DOM changes?** Native EventSource is GET-only and has
no way to update server-side interests in place. Recreating it for each new
visible-ID set is valid, but adds requests, reconnect races, and snapshot/query
work. Since updates are small, best-effort hints and the product is currently
single-user/low-volume, user-scoped delivery plus client-side routing is the
simpler first version. Revisit only if measured event volume warrants narrower
server filtering.

**HTMX's role:** JavaScript uses `htmx.ajax()` to fetch ordinary authenticated
partials. We do not load `htmx-ext-sse`; the SSE transport is native
`EventSource`, and the server continues to render all refreshed HTML through
Tera.

**Transport:** the contract is one stable `EventSource('/events')` connection
per page. The browser uses native `EventSource` in `webui-v2.js`, not
htmx-ext-sse. It parses event JSON, checks
`entity_type`/`entity_id`, and uses `htmx.ajax()` to refresh the matching
ordinary Tera-rendered partial.

The `capture_ids` query parameter only selects the initial snapshot. It is not
a live subscription filter; the server continues to send all events for the
authenticated user.

### 5.2 Direct status indicators (future UI work)

If we later want to display status directly rather than refresh a partial, the
existing JavaScript router can update a status indicator from the received
payload. This is not implemented; current behavior uses ordinary partial
refreshes.

### 5.3 What about the upload flow? (future polish)

The upload already uses a custom XHR with progress. After upload completes, the
server returns `{capture_id, detail_url}`. Under the selected stable-stream
design, no second connection is opened: the page's stream can deliver the new
capture's task hints. Upload notices/status indicators are future client polish.

### 5.4 One SSE channel, many cards

**The question:** the timeline/home page can render 50+ capture cards, and a user
can upload 5 screenshots in quick succession so all 5 are queued/illuminating at
once. How does *one* page-level SSE channel relay the status of *all 5*?

**The target is a multiplexed stream, not a per-card connection.** There is
exactly **one** `EventSource` per page. It carries a
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

**Client side** — the base templates put small `data-*` hints in the DOM for
`web/v2/static/webui-v2.js`:

- `data-sse-mode="feed"` on the index page selects feed refresh behavior.
- `data-sse-mode="detail"` plus `data-capture-id="..."` on the detail page
  tells it which capture detail partial to refresh.
- `data-capture-id` on feed capture-card roots is used to identify entities
  currently rendered in the feed. The detail page uses its body-level
  `data-capture-id` instead.
- `id="capture-card-{id}"` gives the script a stable target for a one-card
  replacement request.

These are ordinary HTML data attributes, exposed as `element.dataset` in
JavaScript. They are routing metadata only: they neither enable SSE by
themselves nor affect HTMX. The script uses `data-sse-mode` to choose page
behavior, opens the stable `/events` stream, parses each `task-status` JSON
payload, and refreshes the matching capture partial if it is rendered. It does
not update subscriptions or reconnect when feed IDs change.

This is a custom `EventSource` listener in `webui-v2.js`, not htmx-ext-sse; the
current templates load HTMX core but do not load the SSE extension. The
EventSource should remain stable as feed DOM changes: only the client-side set
of currently rendered targets changes. An HTMX swap does not warrant closing
and reopening `/events`.

The current implementation uses the single JavaScript router (Pattern B below).
The per-card declarative alternative is historical only; HTMX event triggers do
not inspect JSON payloads for a matching `entity_id`.

**Pattern A — one listener per card (not recommended for precise routing).** Each card could listen for the event:

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

**Pattern B — the selected approach: one JS listener that routes by `entity_id`.** One listener on the shared connection reads
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

### 5.5 Idle close and bounded server lifetime

Each open SSE response occupies a Cloud Run request/concurrency slot, so the
client does not keep the stream open indefinitely. It closes the connection
when the tab is hidden or after five minutes without user interaction; pointer,
keyboard, touch, or wheel activity reconnects a closed stream, and showing the
tab also reconnects it. Reconnection uses the same URL and receives the
connect-time snapshot for the page's original capture IDs.

The server independently closes each response after four minutes, below the
client idle window and any Cloud Run request timeout configured for SSE. A
normal server-side lifetime expiry uses native EventSource reconnect with a
fresh snapshot; transport errors are explicitly closed and retried with
exponential backoff. Reopening also occurs on user activity or when the tab
becomes visible. Do not recreate the stream on HTMX swaps or feed changes.

---

## 6. Deployment / multi-instance correctness

Because the worker can be a different instance than the browser's connection,
`LISTEN/NOTIFY` provides a cross-instance best-effort push hint:

1. **Worker (any instance)** enters the lifecycle through `TaskMaster`; its
  private `TaskRunTracker` writes the task-status row and then publishes a typed
  `TaskStatusEvent` via the injected `ServerEventNotifier` to
  `server_event_channel`.
2. **Every WebUI-enabled instance** runs one dedicated `LISTEN` connection
  (owned by `ServerEventListener`). On a notification, it fans the event out to
  local SSE handlers; each handler filters by owner only. Capture IDs are used
  solely to select the connect-time snapshot.
3. **Catch-up then live stream.** On connect, the SSE handler subscribes to
  notifications, queries the current status for the requested capture IDs, and
  emits those rows before streaming live events. It does not replay transitions
  that are no longer represented by current rows.
4. **Optional polling** may help the UI catch up if `LISTEN/NOTIFY` is
  unavailable; it is not a correctness guarantee.

The local channel is only a delivery optimization for notifications received
from Postgres; neither channel provides retained history.

For Cloud Run specifically: SSE works through the ingress with periodic
keep-alives. Each server response is intentionally capped at four minutes; keep
that below the configured service request timeout so the application, rather
than Cloud Run, normally ends the response. The browser independently closes
idle/hidden streams as described in §5.5.

---

## 7. Why this is "simple, idiomatic, robust, flexible"

- **Simple:** The event schema and Postgres publisher/listener are small,
  separate modules. The client can use HTMX event triggers and one small JS
  router for precise entity fan-out. No JS framework is needed.
- **Idiomatic:** SSE is the canonical HTMX companion; `htmx-ext-sse` is the
  official extension. Axum has first-class SSE support. `LISTEN/NOTIFY` is the
  idiomatic Postgres pub/sub.
- **Robust for the intended scope:** `task_run_status` persists the best
  available current status across restarts. `LISTEN/NOTIFY` gives low-latency
  hints; normal page refresh can help the UI catch up; keep-alives + native
  EventSource reconnect handle flaky connections. This is not a durable change
  log and does not guarantee every transition is delivered. Adaptive lifetime
  remains deferred.
- **Flexible:** The `(task_type, envelope_id, entity_type, entity_id)` model is
  generic — illumination, spark, search-index all flow through the same
  table/channel. Adding a new task type = implement `Task` (with its
  `entity_type`/`entity_id`), write a row + `NOTIFY` + add an
  `hx-trigger="sse:task-status"` line.

---

## 8. Implementation status

**Implemented:** the typed, versioned event model; status notifications from
TaskRunTracker after successful persistence; one per-instance listener/fan-out; authenticated stable SSE route
with a one-time, user-scoped capture snapshot followed by user-wide live events;
and client routing to targeted capture partial refreshes. Also pending:
adaptive idle lifetime, entity-availability producers/consumers, and full-page
status rendering.

| #   | Step                                                                                | Status |
| --- | ----------------------------------------------------------------------------------- | ------ |
| 1   | Integrate `ServerEventNotifier` with task-status writes                             | ✅      |
| 2   | Authenticated `/events` route — initial capture snapshot then user-wide live stream | ✅      |
| 3   | Client routing by entity ID and targeted capture partial refresh                    | ✅      |
| 4   | Client idle/hidden close + bounded server stream lifetime                           | ✅      |

### File changes

| File                               | Change                                                                                            | Status |
| ---------------------------------- | ------------------------------------------------------------------------------------------------- | ------ |
| `src/sse/event.rs`                 | Generic `ServerEvent<E>`, task-status and availability payloads, and wire serialization           | ✅      |
| `src/sse/notifier.rs`              | PostgreSQL `NOTIFY` publisher for typed server events                                             | ✅      |
| `src/sse/listener.rs`              | Dedicated PostgreSQL `LISTEN` receiver and event decoding                                         | ✅      |
| `src/task/taskruntracker.rs`       | private status persistence component; publishes through injected notifier after successful writes | ✅      |
| `src/task/maker.rs`                | selects PostgreSQL notifier and injects it through `TaskMasterBuilder`                            | ✅      |
| `src/bin/dreamscroll_web.rs`       | start WebUI listener and local fan-out                                                            | ✅      |
| `src/webui/v2/maker.rs`            | add `/events`, pass shared event receiver to `WebState`, fingerprint local static assets          | ✅      |
| `src/webui/v2/r_events.rs`         | emit the initial capture snapshot, then user-filtered live updates                                | ✅      |
| `src/webui/v2/r_capture_card.rs`   | authenticated capture-card refresh endpoint                                                       | ✅      |
| `src/webui/v2/r_detail_partial.rs` | authenticated capture-detail partial refresh endpoint                                             | ✅      |
| `web/v2/templates/*.tera`          | `data-sse-mode`, `data-capture-id`, and stable card IDs for client event routing                  | ✅      |
| `web/v2/static/webui-v2.js`        | maintain one stable EventSource; send initial IDs and route all live events by entity ID          | ✅      |
| `Cargo.toml`                       | direct `futures-util` dependency for `stream::unfold` in the Axum SSE handler                     | ✅      |

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
2. **Resolved:** `src/sse/event.rs`, `notifier.rs`, and `listener.rs` are wired
  to task persistence and the UI.
3. **Resolved:** private `TaskRunTracker` publishes a `TaskStatusEvent` after
  successful row inserts/updates for queueing, submission failure, attempt
  start, and attempt outcome. Notification failure is logged but does not affect
  task processing; TaskMaster retains lifecycle policy and ordering.
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
  fan-out are wired. The route emits a one-time current-status snapshot for the
  initial capture IDs, then filters live updates by authenticated owner only.
  The browser keeps one EventSource through feed swaps and routes by entity ID
  to matching DOM targets.
11. **Integrated prototype:** `src/sse` defines `ServerEvent<E>`, the
  `TaskStatusEvent` and `AvailabilityEvent` aliases, typed payloads, and
  versioned JSON serialization. `ServerEventNotifier` publishes to
  `server_event_channel`; `ServerEventListener` owns a dedicated
  `sqlx::postgres::PgListener` connection and decodes the two current event
  types. `webui-v2.js` routes task status by entity ID and refreshes only the
  affected capture card/detail partial while keeping a stable per-page stream.
12. **Product scope clarified:** task-status SSE is informational UI feedback
  only. It tells the user what the backend probably knows and hints that a slim
  page component may need refresh or removal. It is not deterministic pipeline
  logic, a workflow coordinator, or a durable change log. Missed notifications
  are acceptable.
13. **Significant known bug — catch-up refresh amplification:** the initial
  snapshot reports the latest status for each requested capture, including
  long-settled `CompleteSuccess` rows. The browser currently treats every
  refresh-worthy snapshot status like a newly received live transition and
  requests that capture's partial. As task history fills in, a page reload can
  therefore issue one unnecessary partial request per rendered capture. This
  has no known user-visible correctness consequence, but adds avoidable client,
  server, and database work proportional to the number of cards. Keep this open
  until a design distinguishes useful catch-up from live updates; do not hide it
  by weakening the live-event refresh behavior.

## 10. Open questions / follow-ups

- **Catch-up refresh amplification (significant bug):** design a way for the
  client to refresh only when catch-up status indicates content may be stale,
  without refreshing every card whose latest persisted task status is already
  settled. Candidate directions to evaluate include distinguishing snapshot
  events from live events, or comparing task status timestamps against the page
  render time. Preserve refreshes for qualifying live outcomes and account for
  reconnect snapshots; select an approach before implementing it.
- **Coverage gaps from the 2026-09-23 review:** add authenticated route tests for
  capture-card/detail partial access and rendering; assert TaskMaster lifecycle
  status notifications (Queued → InProgress → outcomes, including retry and
  submission failure) through the real publisher; test the SSE snapshot query
  against mixed users/entities/statuses and snapshot/live handoff; and test
  client EventSource reconnect/backoff behavior. Also cover listener failure
  visibility and WebUI startup/shutdown wiring. See `plan/testing.md` for the
  prioritized list. These are follow-up coverage tasks, not blockers for the
  current informational SSE behavior.
- **Spark catch-up:** the initial snapshot currently covers captures only.
  Although feed pages may also show sparks, catch-up for spark entities is
  explicitly deferred; live user-wide status updates continue to be delivered.
  Revisit only if spark status becomes important to the validated use case.
- **SSE payload format:** thin JSON signals remain the recommendation. The
  status field should use the directly serialized `TaskRunStatus`; small HTML
  fragments for direct `sse-swap` remain an optional future use-case.
- **Cloud Run timeout:** verify the configured request timeout comfortably
  exceeds the currently unbounded SSE stream duration, or implement a bounded
  lifetime deliberately if the deployment requires it.
- **Feed changes after connect:** the initial `capture_ids` list is not updated
  when HTMX changes the feed. Newly displayed entities receive future live
  events; a current status for an already-finished task appears on page refresh
  or another targeted partial load.
- **Backfill / bulk tasks (deferred):** status events from backfill currently
  reach the user's stream like any other event. Suppressing or distinguishing
  them is out of scope; create a separate `plan/backfill.md` and feature branch
  when that work starts. A `background` annotation may be considered then.
- **Capture lifecycle publishing:** the `AvailabilityEvent` wire type already
  models `available`/`deleted` operations for any entity type. Publishing these
  updates remains future work and should use a deliberate source/producer; do
  not shoehorn lifecycle events into `task_run_status`. The entity-scoped
  snapshot selection and single-SSE-connection design are intended to
  accommodate this later. Its operation hints can use the same stable per-user
  stream.
