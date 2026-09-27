# Task Status Timing & Progress Bar — Design Plan

**Status:** Timing persistence implemented; SSE and UI remain intentionally deferred.
**Scope:** Add persisted timing facts to `task_run_status`, expose them through task-status reads/SSE, and define the data needed for an estimated illumination progress indicator.

> Related: `plan/task-status.md`, `plan/sse.md`, `src/task/taskruntracker.rs`,
> `src/task/taskmaster.rs`, `src/model/task_run_status.rs`, and `src/sse/event.rs`.

## 1. What exists today

- `task_run_status.created_at` is set when `create_run` inserts the initial `Queued` row. For the current implementation, this is the timestamp when the run first entered the queue, although its name does not communicate that intent.
- `updated_at` is changed on every status update, but it is not a lifecycle timestamp: it can represent a retry transition, a late update, or any future metadata update.
- `begin_attempt` changes `Queued`/`ErrorWillRetry` to `InProgress` and increments `attempts`.
- `finish_attempt` changes `InProgress` to `CompleteSuccess`, `ErrorWillRetry`, or `CompleteFailure`.
- Status events currently carry status, attempts, run, and routing identity, but no timing information.

## 2. Proposed persisted facts

Add these columns to each `(envelope_id, run)` row:

| Field | Type | Meaning |
| --- | --- | --- |
| `processing_started_at` | `TIMESTAMPTZ NULL` | When the **most recent attempt** entered `InProgress`; null before any worker starts. |
| `last_error_duration_ms` | `BIGINT NULL` | Duration of the most recent failed attempt, from its processing start until `finish_attempt`; null if no attempt has failed. |
| `success_duration_ms` | `BIGINT NULL` | Duration of the successful attempt; null until this run succeeds. |

Keep `created_at` as the first-queued timestamp for now. Consider a later rename or an API alias such as `queued_at` if the distinction becomes confusing; a rename is not required to implement the feature.

### Important retry semantics

`processing_started_at` should mean **most recent attempt**, not first-ever processing. A retry is a new processing interval and must reset this timestamp in `begin_attempt`. The two duration fields preserve the useful terminal/history facts across retries:

- On every failed attempt, overwrite `last_error_processing_duration_ms`.
- On success, set `success_processing_duration_ms` for the successful attempt.
- Do not clear the prior error duration on success; it can help diagnose a run that succeeded only after retries.
- A run that is permanently failed still has `last_error_processing_duration_ms` for its final error.

If product instead wants the first worker-start timestamp, add a separate immutable `first_processing_started_at`; do not overload the proposed field with two meanings.

## 3. Timing boundaries

Use database timestamps for persisted lifecycle boundaries so all workers/instances share one clock:

1. `create_run`: `created_at` = queued-at timestamp.
2. `begin_attempt`: set `processing_started_at = CURRENT_TIMESTAMP` in the same update that writes `InProgress`.
3. `finish_attempt`: compute elapsed time from the persisted `processing_started_at` to `CURRENT_TIMESTAMP`, then write the appropriate duration column in the same database update.

The duration should be an integer number of milliseconds (or microseconds if measurement precision is needed later). Milliseconds are sufficient for user-facing progress and avoid exposing database interval serialization details on the JSON wire.

### Missing-start behavior

If `finish_attempt` finds `processing_started_at IS NULL`, do not invent a duration. Persist the status transition, leave the duration null, and log a warning. This protects data quality if a stale/malformed delivery reaches the finish path.

## 4. API and SSE shape

Expose timing fields as part of the task status model/event, using names that distinguish timestamps from durations. Proposed JSON fields:

```text
queued_at                         // current created_at, or an explicit alias
processing_started_at             // nullable timestamp for current/last attempt
queue_wait_duration_ms            // derived when processing has started; nullable otherwise
last_error_processing_duration_ms // nullable
success_processing_duration_ms    // nullable
```

`queue_wait_duration_ms` does not need to be persisted initially: derive it as
`processing_started_at - created_at` for a run that has started. If the UI needs a stable value after later retries, add a separate persisted `first_processing_started_at` or `queue_wait_duration_ms`; do not derive it from the mutable most-recent start timestamp.

SSE is intentionally deferred until the persistence and query behavior is proven. The progress bar is a UI concern, not a database field or task-status concern.

The existing `TaskStatusEvent::from_envelope` path currently receives only status/attempts/timestamp. It will need either the timing values or a status-row/snapshot object so notifications contain the same timing facts as polling responses. Avoid making notifier code query the database after every write.

## 5. Tracker/TaskMaster implementation shape

Prefer lifecycle-specific tracker operations over a generic status setter:

- `create_run`: unchanged status behavior; confirms the queued timestamp is captured.
- `begin_attempt`: a tracker method that atomically sets `InProgress`, increments attempts, and sets `processing_started_at = CURRENT_TIMESTAMP`.
- `finish_attempt`: a tracker method that atomically sets the outcome and computes the duration from the stored processing start. It should update the error duration for every error and the success duration for success.

This keeps timing invariants next to the existing attempts/status invariants and avoids relying on Rust `Instant` across workers. `TaskMaster` remains the lifecycle policy boundary. The implementation uses PostgreSQL `CURRENT_TIMESTAMP` for both the start timestamp and the finish calculation, with `EXTRACT(EPOCH ...) * 1000` cast to `BIGINT`.

## 6. Estimated progress bar

These timings provide a useful first estimate but not true percentage completion:

- **Queued:** show indeterminate progress, or elapsed queue time against a queue-wait estimate.
- **InProgress:** estimate remaining time from historical successful processing durations for the same task type, ideally using a rolling median/percentile rather than the last run.
- **Retrying:** show that the attempt failed and is queued again; do not reset the overall UI state to zero.
- **Success/failure:** show 100%/terminal state.

A single `success_processing_duration_ms` on the current run is not enough to estimate progress for that same run before completion. The estimate needs an aggregate over prior completed runs (or a configured task-type baseline). This should be a separate query/model concern rather than another status enum value.

For the initial UI, prefer an indeterminate bar with a human-readable elapsed time. Add a determinate estimate only after collecting enough successful durations and deciding whether the estimate is per task type, per user, or global.

## 7. Tests to add

### Tracker/DB tests

- New runs have a non-null queued timestamp and all timing fields null.
- Beginning an attempt sets `processing_started_at` and increments attempts.
- Finishing successfully writes a positive/non-negative success duration.
- Failing an attempt writes the most recent error duration.
- A retry resets `processing_started_at` and overwrites the error duration on the next failure.
- Success after an error preserves the last error duration and sets success duration.
- A missing processing start leaves duration null and does not prevent the status update.
- A second run has independent timing fields.

Avoid brittle exact timestamp assertions; assert ordering and reasonable duration bounds.

### SSE/API tests

- Status events include the new nullable timing fields.
- Queued, in-progress, retrying, success, and failure payloads have the expected null/non-null combinations.
- Existing clients/tests that deserialize task events remain compatible or are updated with the schema-version decision.

### Estimate tests

Once an estimate query is designed, test that it has no estimate with insufficient history and that it is based only on completed successes (not queue wait or failed-attempt duration).

## 8. Open decisions for discussion

1. Should `processing_started_at` mean the first-ever processing start or the most recent attempt? This plan recommends most recent attempt and, if needed, a separate immutable first-start field.
2. Is `created_at` acceptable as the queued timestamp, or do we want an explicit `queued_at` column/API field for clarity?
3. Milliseconds or microseconds for duration values? Recommendation: milliseconds.
4. Should error duration represent the most recent failed attempt (recommended) or only the final/permanent failure?
5. Should timing be present in SSE immediately, or should this first land in DB/query responses?
6. For progress estimation, should the first version be indeterminate, or do we want to design historical-duration aggregation now?
7. What should happen to old rows after the schema sync adds nullable columns? Recommendation: leave them null; no migration/backfill is needed under the repository's prototype policy.

## 9. Suggested incremental implementation order

1. Agree on the timestamp/duration semantics above.
2. Add nullable timing columns to the SeaORM model and update `plan/task-status.md`'s schema/file map.
3. Add lifecycle-specific tracker writes and TaskMaster integration. **Done.**
4. Add DB-backed tracker tests.
5. Add timing to status snapshots. SSE remains a separate follow-up.
6. Add an indeterminate progress UI using current status and elapsed time.
7. Separately design historical timing aggregation for a determinate estimate.
