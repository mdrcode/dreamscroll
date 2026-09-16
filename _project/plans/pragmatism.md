# Pragmatism — tolerated trade-offs

**Status:** living document. Last updated 2026-09-16.

## Philosophy

Dreamscroll is a personal project whose primary risk is **not** technical. The
primary risk is that the app isn't useful enough to justify its own existence.
Every hour spent hardening a race condition that only manifests under
multi-user concurrency is an hour not spent validating the core use case.

So: **we deliberately tolerate a set of known issues** in order to move fast on
product validation. This document is the ledger of those trade-offs, so that:

1. We don't rediscover them repeatedly and re-litigate whether they matter.
2. We don't forget them — each entry has a **revisit trigger** describing when
   it stops being acceptable.
3. We can tell the difference between "we didn't notice" and "we chose this."

**The bar for tolerating something:** it must be primarily a *throughput, cost,
or code-cleanliness* concern, with no severe negative user-facing effect. If an
issue can corrupt user data, leak another user's data, or silently lose work in
a way the user would notice, it does **not** belong here — it gets fixed.

**The bar for fixing something now:** it's cheap, unambiguous, and doesn't
distract from product work. (Several items below were fixed on exactly that
basis and are recorded as resolved for context.)

## How to use this doc

- When you find an issue, ask: *does this have a severe user-facing effect?*
  - **Yes** → fix it, or file it as a real bug.
  - **No** → add it here with a revisit trigger, and move on.
- When a revisit trigger fires, promote the entry to a real task.
- When you fix something, mark it resolved rather than deleting it — the
  history of what we chose to tolerate is itself useful.

---

## Tolerated trade-offs

### Concurrency

These are all "correct at one user, wrong under concurrency." They are the
largest category, and the least urgent, because the app is single-user today.

| Issue                                                                              | Effect                                                                                                                                                                                                                                                                                       | Why tolerated                                                                                                                                            | Revisit trigger                                                                            |
| ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| **`TaskStatusRecorder::record` is SELECT-then-INSERT, not an atomic upsert**       | Two concurrent writers can both see "no row" and both INSERT. With `UNIQUE(envelope_id)` now present, the loser gets a unique-violation error instead of silently duplicating.                                                                                                               | Requires two writers racing on the *same* task envelope, which needs genuine concurrency on one capture.                                                 | First time a unique-violation is observed in logs, or when a second user is added.         |
| **TOCTOU idempotency guards in `logic/illuminate.rs` and `logic/search_index.rs`** | Two workers can both pass the "already done?" check and both do the work. For illuminate this means a **duplicate LLM call** (cost + latency) and possibly a duplicate illumination row. For search_index it's a redundant vector upsert (benign — the store upserts on a deterministic id). | Purely cost/throughput. The read path already tolerates duplicate illuminations (`get_captures_need_search_index` dedupes; templates render `\| first`). | Duplicate LLM spend becomes noticeable, or duplicate illuminations start confusing the UI. |
| **No `ORDER BY` on the incomplete-status queries**                                 | Row order is non-deterministic.                                                                                                                                                                                                                                                              | Nothing iterates the results yet.                                                                                                                        | When the SSE handler or a UI view iterates them.                                           |

> **Note on the illuminate guard:** the fix is coupled to the deferred rerun
> design. A `UNIQUE(capture_id)` constraint on `illuminations` is the clean fix
> *today*, but would have to be dropped if reruns append new illuminations
> rather than replacing. Decide rerun semantics first.

### Cost / throughput

| Issue                                                                                                  | Effect                                                                                                                                                      | Why tolerated                                                                                | Revisit trigger                                                            |
| ------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| **`api/user/client.rs` swallows `submit_illumination` errors** (logs a warning, still returns success) | A capture can be uploaded but never illuminated/indexed, with no user-visible signal.                                                                       | The upload itself succeeded, which is what the user cares about. Failure is visible in logs. | A user notices a capture that never got illuminated.                       |
| **`CloudTaskQueue` posts to a hardcoded dummy URL**                                                    | Correctness depends entirely on each queue being created with `--http-uri-override` + OIDC overrides. A misconfigured queue silently posts to a bogus host. | Queue config is documented in `_project/gcloud/cloud_task_queue.md` and set up once.         | A queue is created without the overrides, or this bites during a redeploy. |

### Security / defense-in-depth

| Issue                                                             | Effect                                                                                                                                                            | Why tolerated                                                                                                                                         | Revisit trigger                                                                                |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| **Envelope `user_id` is not validated against the capture owner** | `logic::spark::exec` derives `user_id` from the captures and never compares it to `envelope.user_id`. `api/service/get_capture.rs` is explicitly not user-scoped. | The webhook routes are protected by Cloud Run OIDC, so an attacker can't reach them. This is defense-in-depth against our own bugs, not an open door. | Adding a second user, or any path where a webhook could be invoked with a mismatched envelope. |
| **`query_snapshot(envelope_id)` is not user-scoped**              | `envelope_id` embeds `user_id`, so it's *implicitly* scoped, but the predicate isn't enforced.                                                                    | Internal-only call, used by `begin_attempt`. The public queries *are* explicitly user-scoped.                                                         | If it ever becomes reachable from a request handler.                                           |

### Operational

| Issue                                                 | Effect                                                                                                                    | Why tolerated                                        | Revisit trigger                                                             |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | --------------------------------------------------------------------------- |
| **No retention / vacuum policy on `task_status`**     | The table grows forever.                                                                                                  | Tiny table, single user.                             | Row count becomes non-trivial, or query latency degrades.                   |
| **No heartbeat / timeout for stuck tasks**            | A task enqueued but never picked up (queue dropped, worker crash) stays `Queued` forever and looks active.                | Requires an actual lost task, which hasn't happened. | First observed stuck task, or when the SSE UI makes it visible to the user. |
| **Composite index on `task_status` deferred**         | The incomplete-status queries scan on a single-column `entity_id` index.                                                  | Single-user app, tiny table.                         | Query latency degrades, or the table grows past a few thousand rows.        |
| **`submit_inner` records `Queued` before enqueueing** | If enqueue fails, the row is orphaned at `Queued` with no task behind it — indistinguishable from a genuinely stuck task. | Same as above; enqueue failures are rare and logged. | Same as the stuck-task trigger.                                             |

### Code cleanliness

| Issue                                                               | Effect                                                                                                                                                   | Why tolerated                                                                                              | Revisit trigger                                         |
| ------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| **`/_wh/cloudtask/illuminate` route is unused**                     | The capture-create path submits `IlluminationTask` through the illumination queue, so the route has no live caller.                                      | Kept deliberately — it's the entry point for the future backfill and re-run-with-a-new-model/prompt flows. | When backfill or reruns are implemented.                |
| **`TaskEnvelope.task` is `Option<T>` but never `None` in practice** | All four handlers carry a `task: None` → 400 guard that can never fire, since the queue always serializes the payload.                                   | Kept deliberately: it's forward-compatible with a future payload-less query handle.                        | If the `Option` never earns its keep, make it required. |
| **No sub-step visibility for composite work**                       | `logic/illuminate::exec` runs illumination + search-indexing under one status row, so a client sees only `illuminate`, never "illuminating vs indexing". | The user sees a single "working…" state, which is arguably better UX anyway.                               | If the UI wants to show distinct sub-steps.             |
| **"Two-owner rule" is slightly aspirational**                       | Docs say only `TaskMaster` (writes) and `TaskWatcher` (reads) touch `task_status`, but `TaskStatusRecorder` also touches it directly (shared by both).   | Harmless — the recorder *is* the shared persistence utility.                                               | If a third caller appears.                              |

---

## Deferred to dedicated sessions

These aren't "tolerated" so much as "explicitly out of scope for now, with a
plan to do them properly."

| Topic                        | Notes                                                                                                                                                                                                                                                                                                                                |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Backfill / bulk tasks**    | The `background` flag was removed (never populated). Backfill handling — marking tasks as bulk, surfacing progress, an admin view — plus the `user_id` mis-attribution and the global candidate query in `get_captures_need_search_index`, all get a dedicated plan-and-branch session. See `plans/sse-task-status.md` §4.3 and §13. |
| **Reruns**                   | `run_id` was removed. Re-introducing a run dimension interacts with task identity (deterministic `envelope_id`), the illuminate idempotency guard, and the `illuminations` schema. See `plans/sse-task-status.md` §7.                                                                                                                |
| **Capture lifecycle events** | Explicitly out of scope for the task-status phase; gets its own mechanism. See `plans/sse-task-status.md` §13.                                                                                                                                                                                                                       |

---

## Resolved (kept for context)

Fixed because they were cheap, unambiguous, and didn't distract from product
work. Recorded here so the ledger is complete.

| Issue                                                                                     | Resolution                                                                                                              |
| ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `task_status` had no entity linkage                                                       | Added `entity_type` + `entity_id` (indexed) and the incomplete-status query APIs.                                       |
| `TaskEnvelope` `Debug` could panic on a UTF-8 boundary                                    | Now takes a char-boundary-safe prefix.                                                                                  |
| Retry/exhaustion policy undefined (`attempts` hardcoded, `ErrorFinal` never written)      | Full policy: `task_max_attempts`, `ApiError::is_retryable()`, `AttemptOutcome`, and the ack-on-exhaustion HTTP mapping. |
| `query_status` ignored `user_id`                                                          | Removed entirely (no callers).                                                                                          |
| `background` always `false`                                                               | Field removed.                                                                                                          |
| `ErrorWillRetry` returned `503`                                                           | Changed to `500` — `503` triggers Cloud Tasks *queue-wide* congestion throttling.                                       |
| `begin_attempt` could resurrect a completed task                                          | Now returns `None` for already-`Completed` tasks, which the handler acks.                                               |
| Builder default `max_attempts` was 1                                                      | Now 3, mirroring the config default.                                                                                    |
| Assorted doc drift (`from_task` → `new`, stale "terminal" concept, `task_id` in examples) | Corrected.                                                                                                              |
