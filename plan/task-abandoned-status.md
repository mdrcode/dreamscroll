# Abandoned task-run status

**Status:** Deferred — trigger-gated. Local-only problem today; production self-heals.

`CompleteFailure` (status 5) is the only terminal failure, so it carries two different meanings:
the provider failed, and *an operator closed the row*. The two repair verbs
(`dreamscroll_db task fail_stale`, `task drain_queued`) do the latter. This doc is the case for a
seventh status, `Abandoned`, and the traps to avoid if it ever lands.

## Why it is deferred

Production does not have the problem. Cloud Tasks redelivers a failed delivery, so a run whose
instance died is retried and reaches a real outcome. Rows only strand under the in-memory
`LocalTaskQueue`, where a restart drops queued work while `task_runs` keeps the status — see the
matching row in `plan/pragmatism.md`. Operator repair is therefore a localdev operation today.

## Trigger

A production run needs abandoning by hand, **or** operator repairs start appearing in failure
reports or latency percentiles where a reader would reasonably read them as provider failures.

## What intent costs today

An abandoned run has no typed trace of why: `attempts` 0 or 1, `last_error_code` and
`last_error_duration_ms` null, and the reason lives only in `last_error_message` prose
(`"drained: was queued…"`, `"abandoned: was in_progress…"`). That column is documented as the
most recent attempt's error chain, and the `drained:`/`abandoned:` prefixes are a convention nothing
enforces. Consumers that care must string-match.

## Shape

- Append `Abandoned = 6` — **never renumber**. `status_code` is a plain int with no CHECK
  constraint, so renumbering silently relabels history.
- `as_str` → `"abandoned"`; `is_in_flight` → `false` so `decide_next_run` lets the next submission
  claim a new run number; terminal, not retried.
- Schema sync needs nothing.

## Traps, in hazard order

1. **Rolling deploy.** `from_i32` is strict and `backfill::status()` propagates it
   (`map_err(ApiError::internal)?`), so an old revision reading a `6` written by a new one fails that
   endpoint with a 500. Ship tolerant readers first, writers in a later step. (`decide_next_run` and
   `sse::from_row` are benign — one falls through to "allow", the other drops the event.)
2. **Fourteen SQL literals encode "failed" as `status_code = 5`** in `stats/failures.rs` and
   `stats/queue.rs`. Decide per site: exclude abandoned runs from failure latency (they never
   executed, so they measure nothing), include them in `backfill::failures()` (work vanished and the
   report must not read clean), and let `queue failures` exclude them by default behind an
   `--include_abandoned` flag.
3. **`decide_next_attempt` guards only success and submission-failure**, so a stray redelivery of an
   abandoned run would bump `attempts` and resurrect it to `InProgress`, undoing the operator's
   decision. Add it to the do-not-resurrect set.
4. **The UI names statuses.** `web/v2/static/webui-v2.js::isRefreshWorthyTaskStatus` must include
   `abandoned`, or an abandoned capture keeps its spinner until a manual reload; it also needs an
   indicator branch and a chip style.
5. `webhook/http_status_for_task_run.rs` is an exhaustive match, so the compiler forces the
   non-retry decision (`OK`).

## History

73 dev rows were closed as `CompleteFailure` (3 `abandoned:`, 70 `drained:`) before this status
existed. If it lands, reclassify by note prefix — `UPDATE … WHERE last_error_message LIKE
'drained:%'` — as a one-off dev-DB write, or leave the history as it is and document the seam.
