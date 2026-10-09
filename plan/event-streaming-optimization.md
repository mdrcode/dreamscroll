# Event-streaming optimization

**Status:** exploratory. **Last updated:** 2026-10-06.

## Problem

Every new `/events` connection performs the initial task-status catch-up query for
its requested capture IDs. This happens on the first page load and on every
reconnect, even when the same page has already received the relevant statuses
and nothing changed.

The browser keeps one native `EventSource` object. Transport errors and normal
server stream closure use EventSource's native retry, which reuses the original
URL and catch-up IDs. The client constructs a new source only after its
heartbeat watchdog replaces a stale `OPEN` source, or after an idle close when
activity resumes (including upload's `ensureConnected` call when no source is
present). Visibility alone does not replace a healthy source; Safari may still
suspend background networking. Every resulting HTTP connection executes a
fresh catch-up query, so repeated requests still produce query volume roughly
linear in connections rather than actual task-status changes.

This is currently accepted as prototype behavior, but it creates avoidable
PostgreSQL churn and can become significant as connection frequency or the
number of requested captures grows.

## Current semantics

`src/webui/v2/r_events.rs`:

- authenticates the SSE request;
- subscribes to the local server-event fan-out;
- queries the latest status for the requested `capture_ids`;
- emits those rows as catch-up `task-status` events;
- then forwards live user-wide task-status events.

The `capture_ids` parameter selects the initial catch-up set only. It is not a
long-lived server-side subscription filter.

SSE event envelopes currently have a generic `timestamp` field. For
`TaskStatusEvent::from_row`, this field is populated from
`task_runs.updated_at`:

```rust
timestamp: row.updated_at
```

However, `ServerEvent::new` accepts an arbitrary timestamp and availability
events are constructed independently. Therefore, the timestamp is effectively
a persisted status-row freshness timestamp for row-derived task-status events,
but it is not a universally enforced `updated_at` or event-log sequence for
every server event type.

## Candidate optimization: timestamp cursor

Have the browser track the greatest task-status timestamp it has received and
send it on reconnect:

```text
/events?capture_ids=12,34&after=2026-09-28T18:42:10.123456Z
```

The server would filter catch-up rows to statuses newer than `after`. The first
connection would omit `after` and retain current behavior. Reconnects with no
newer rows would emit no catch-up events.

The browser must track the maximum timestamp, not merely the last event
received, because events can arrive in an order that does not match their
freshness ordering:

```text
last_seen_status_timestamp = max(last_seen_status_timestamp, event.timestamp)
```

The server must subscribe to live notifications before querying catch-up rows,
so an update occurring while the query runs is not silently missed.

## Benefits

- Small incremental change to the existing latest-state design.
- Avoids repeatedly sending unchanged catch-up events.
- Avoids repeated client-side progress handling and card-refresh work for those
  unchanged events.
- Does not require a durable event log or replay protocol.
- Preserves the current semantics: catch-up returns current rows, not every
  historical transition.

## Risks and unresolved questions

### Timestamp meaning

Should the generic envelope field `timestamp` be used as the cursor, or should
we add an explicit task-status field such as `status_updated_at`? Using the
existing field is simpler, but its meaning is not enforced for generic events.
The cursor should be documented as valid only for row-derived task-status
events unless the event model is tightened.

### Timestamp collisions

A timestamp-only cursor with a strict `>` comparison can skip a row if multiple
updates share the same timestamp precision. PostgreSQL timestamps commonly have
microsecond precision, making collisions unlikely but not impossible.

A stronger cursor would be a compound value such as:

```text
(updated_at, task_runs.id)
```

The SQL ordering and comparison would need to use both values consistently.

### Missed transitions

The cursor would not replay transitions missed during a disconnect. It would
only return the latest current row for each relevant task, matching the current
best-effort stream semantics. This is acceptable only if clients need current
state rather than a complete transition history.

### Cursor lifetime

The browser may retain a cursor across reconnects, but should reset it on a
full page load unless there is a deliberate cross-page persistence policy. A
cursor must not be allowed to suppress the initial state for newly rendered
capture cards that were not part of the previous page.

### Query cost remains

A timestamp cursor eliminates duplicate catch-up rows, but the server may still
execute a catch-up query on every connection. To reduce database work itself,
we may need a cheap freshness check, a per-user/page cache, a durable sequence,
or a different endpoint behavior when no cursor-relevant changes exist.

## Possible designs

### A. Timestamp-only cursor

Add an optional `after` timestamp to `/events` and filter catch-up rows by
`updated_at > after`.

- Lowest implementation cost.
- Collision risk.
- Still performs a database query on reconnect.

### B. Compound `(updated_at, id)` cursor

Add an optional cursor containing both the row update timestamp and the row ID.
Use a lexicographic comparison and ordering.

- More reliable than timestamp-only filtering.
- Still compatible with current current-state semantics.
- More wire-format and SQL complexity.

### C. Monotonic event/status sequence

Introduce a durable sequence for status changes and use it as the cursor.

- Strong ordering and no timestamp collision ambiguity.
- Requires schema/API changes and clearer event-log semantics.
- Still does not replay transitions unless historical events are retained.

### D. Freshness shortcut or cache

Avoid the full catch-up query when the server can establish that nothing relevant
changed since the cursor, potentially using a cheap aggregate, per-user cache,
or a database-side maximum update timestamp.

- Addresses database churn directly.
- Adds cache invalidation or additional query/design complexity.
- Must account for multiple application instances and Cloud Run topology.

## Recommended investigation order

1. Confirm the exact catch-up query and its cost under reconnect-heavy behavior.
2. Decide whether task-status `ServerEvent.timestamp` should be formally defined
   as `task_runs.updated_at`.
3. Determine whether timestamp collisions are acceptable for the prototype.
4. If yes, prototype a timestamp-only cursor and test reconnect/catch-up races.
5. Measure whether eliminating duplicate payloads is sufficient or whether the
   database query itself remains a material cost.
6. If the query remains expensive, evaluate a compound cursor, durable sequence,
   or freshness shortcut.
7. Update `plan/sse.md` and the task-status timing plan after the design is
   selected.

## Non-goals for this plan

- Building a durable event replay log.
- Guaranteeing delivery of every intermediate task transition.
- Changing the existing user-wide live-event routing model.
- Replacing SSE with polling or a different transport.
