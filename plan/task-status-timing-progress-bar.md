# Task Status Timing & Progress Bar — Design Plan

**Status:** Timing persistence, update-in-place aggregate measures, SSE metadata, and client-side estimated progress implemented.
**Scope:** Add persisted timing facts to `task_run_status`, expose them through task-status reads/SSE, and define the data needed for an estimated illumination progress indicator.

> Related: `plan/task-status.md`, `plan/sse.md`, `src/task/taskruntracker.rs`,
> `src/task/taskmaster.rs`, `src/model/task_run_status.rs`, and `src/sse/event.rs`.

## 1. What exists today

- `task_run_status.created_at` is set when `create_run` inserts the initial `Queued` row. For the current implementation, this is the timestamp when the run first entered the queue, although its name does not communicate that intent.
- `updated_at` is changed on every status update, but it is not a lifecycle timestamp: it can represent a retry transition, a late update, or any future metadata update.
- `begin_attempt` changes `Queued`/`ErrorWillRetry` to `InProgress` and increments `attempts`.
- `finish_attempt` changes `InProgress` to `CompleteSuccess`, `ErrorWillRetry`, or `CompleteFailure`.
- Status events carry status, attempts, run, routing identity, processing start,
  and optional aggregate timing estimates.

## 2. Proposed persisted facts

Add these columns to each `(envelope_id, run)` row:

| Field                    | Type               | Meaning                                                                                                                      |
| ------------------------ | ------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `processing_started_at`  | `TIMESTAMPTZ NULL` | When the **most recent attempt** entered `InProgress`; null before any worker starts.                                        |
| `last_error_duration_ms` | `BIGINT NULL`      | Duration of the most recent failed attempt, from its processing start until `finish_attempt`; null if no attempt has failed. |
| `success_duration_ms`    | `BIGINT NULL`      | Duration of the successful attempt; null until this run succeeds.                                                            |

Keep `created_at` as the first-queued timestamp for now. Consider a later rename or an API alias such as `queued_at` if the distinction becomes confusing; a rename is not required to implement the feature.

### Important retry semantics

`processing_started_at` should mean **most recent attempt**, not first-ever processing. A retry is a new processing interval and must reset this timestamp in `begin_attempt`. The two duration fields preserve the useful terminal/history facts across retries:

- On every failed attempt, overwrite `last_error_duration_ms`.
- On success, set `success_duration_ms` for the successful attempt.
- Do not clear the prior error duration on success; it can help diagnose a run that succeeded only after retries.
- A run that is permanently failed still has `last_error_duration_ms` for its final error.

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
processing_started_at       // nullable timestamp for current/last attempt
estimated_duration_ms_p50   // optional aggregate estimate used by the client
```

`queue_wait_duration_ms` does not need to be persisted initially: derive it as
`processing_started_at - created_at` for a run that has started. If the UI needs a stable value after later retries, add a separate persisted `first_processing_started_at` or `queue_wait_duration_ms`; do not derive it from the mutable most-recent start timestamp.

Task-status SSE events include optional `processing_started_at` and aggregate
estimate metadata. The browser uses p50 to animate an estimated processing bar
locally; the server does not stream timer ticks.

`TaskStatusEvent::from_envelope` accepts the complete `task_run_status` row and
an optional `TaskTimingEstimate`. `TaskStatusEvent::from_row` supports snapshot
events without an envelope. This keeps row-to-payload construction in the SSE
event module while database access remains in `task_timing.rs`.

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

A single `success_duration_ms` on the current run is not enough to estimate progress for that same run before completion. The estimate uses aggregate measures from prior completed runs. This is separate from the status enum.

The current UI renders progress inside each capture card, so multiple
simultaneously processing captures have independent bars. It uses p50 when
available, caps live progress at 92%, and switches to an indeterminate
long-tail state after the estimate is exceeded. It falls back to an
indeterminate bar when no estimate exists.

## 6.1 Empirical duration estimator proposal

The next iteration should use recent successful processing durations as an empirical
estimate rather than hard-coded lifecycle percentages.

### Estimate population

- Keep the most recent 100 successful `success_duration_ms` values.
- Scope the population by `task_type` initially (`illumination`), not by user or
	entity. This gives enough samples sooner and avoids leaking user-specific data.
- Exclude failed attempts, queue wait, and the current in-progress run.
- Prefer the median as the central estimate. A mean is too sensitive to an
	occasional slow LLM/API call. Later, a p75 or p90 can provide a more honest
	"likely complete by" bound.
- Require a minimum sample count before using a determinate estimate (for
	example, 5 or 10). Until then, retain the indeterminate bar.

The query would conceptually be:

```sql
SELECT success_duration_ms
FROM task_run_status
WHERE task_type = $1
	AND status_code = <CompleteSuccess>
	AND success_duration_ms IS NOT NULL
ORDER BY updated_at DESC
LIMIT 100;
```

PostgreSQL calculates average and ordered-set percentiles over the bounded
result set.

### Refresh policy

For the first implementation, recompute the affected estimate after each
relevant task-run update. The source query is bounded to the most recent 100
relevant rows, and the result is stored in a single update-in-place measure row
for use by clients. `InProgress` refreshes `queue_wait`; `CompleteSuccess`
refreshes `processing_successful`; other statuses do not refresh a measure.

The processing estimator derives its population only from successful runs.
Adaptive refreshing, debouncing, and modulo/coin-flip sampling are explicitly
deferred.

### Measure storage

Use the database-backed `task_run_timing` measure keyed by
`(task_type, operation_type)`, where operation type is the strongly typed
`TaskTimingMeasure` value `queue_wait` or `processing_successful`. It stores `sample_count`, `avg`,
`p50`, `p75`, `p90`, and `updated_at`. It is aggregate metadata, not a lifecycle
fact about one run, and is updated in place rather than recorded as history.
Each relevant task-run update selects the latest 100 source rows, computes the
aggregates in PostgreSQL, and upserts the corresponding measure row. The
operations are `queue_wait` and `processing_successful`; the source durations are respectively
`processing_started_at - created_at` and `success_duration_ms`.

The measure model is `src/model/task_run_timing.rs` and is synchronized with
the rest of the schema. It stores `sample_count`, `avg_duration_ms`,
`p50_duration_ms`, `p75_duration_ms`, `p90_duration_ms`, and `updated_at`.

### Relaying the estimate to clients

Task-status SSE payloads include the following optional estimate metadata:

```text
estimated_duration_ms_p50
estimate_sample_count
```

The existing status event can carry this because the client needs the estimate
when the run enters `InProgress`. The event does not need to stream elapsed time
every second. The browser already knows the `processing_started_at` timestamp (or
can begin a local timer when it receives `InProgress`) and animates locally.

The server attaches the current measure to lifecycle status events. It does not
emit synthetic status transitions for measure refreshes.

The server only exposes `estimated_duration_ms_p50` when the timing aggregate
has at least five successful samples. The aggregate row and sample count remain
database-side observability data; the sample count is not sent to the browser.

### Client-side calculation

When the client receives `InProgress`:

1. Record the local start time from the server timestamp.
2. Read `estimated_duration_ms_p50` from the event.
3. Animate a determinate bar as `elapsed / estimated_duration_ms_p50`, capped below 100% (for
	 example at 92%) while work is still running. Keep this state scoped to the
	 capture card so concurrent captures do not share progress.
4. Switch to a subtle indeterminate/slow tail after the estimate is exceeded;
	 never move backward or claim certainty.
5. Set 100% only for `CompleteSuccess` or `CompleteFailure`.

This gives a live bar correlated with empirical history while honestly handling
long-tail tasks. Queue time should remain a separate queued state and should not
consume processing progress.

### Open design choice: task versus attempt estimate

The first version should estimate the processing duration of one attempt. A retry
can reset the attempt bar while retaining a user-facing message such as
"Retrying (attempt 2)". If retries become common, add an overall run-level
estimate that combines expected attempt count and retry probability; do not hide
that complexity inside the basic progress percentage.

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
5. Add timing to status snapshots and extend SSE with optional estimator metadata. **Done.**
6. Replace the milestone-only sketch with a client-side elapsed/estimate bar. **Done.**
7. Add bounded historical aggregation and refresh update-in-place measures on relevant status updates. **Done.**
