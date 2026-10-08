# Pragmatism — tolerated trade-offs

**Status:** living document. Last updated 2026-10-08.

> **See also:**
> - `task-status.md` — the task framework (implemented).
> - `sse.md` — the implemented SSE delivery layer.
> - `testing.md` — the two-tier test model.
> - `topology_and_throughput.md` — connection/concurrency budgets.

## Philosophy

Dreamscroll is an exploratory  project whose primary risk is **not** technical.
The primary risk is that the app isn't useful enough to justify its own
existence. Every hour spent hardening a race condition that only manifests
under multi-user concurrency is an hour not spent validating the core use case.

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

| Issue                                                                        | Effect                                                                                                                                                                                                                                      | Why tolerated                                                                                                                                                                                                                                                   | Revisit trigger                                                                                       |
| ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| **No idempotency guards in `logic/illuminate.rs` / `logic/search_index.rs`** | A retry re-calls the LLM and re-embeds, and can leave a duplicate illumination row. Two concurrent workers could both do the work.                                                                                                          | Purely cost/throughput, and both calls are already tolerated as duplicable. The read path shows only the most recent illumination, so duplicates never surface in the UI. Guards were *removed* (2026-09-16) rather than made rerun-aware — see the note below. | Duplicate LLM spend becomes noticeable on retries, or duplicate illuminations start confusing the UI. |
| **An in-flight search-index task can miss a newer illumination**             | If re-illumination occurs while indexing is in flight, that task may have already loaded the prior illumination. Beacon refuses a duplicate in-flight index submission, so the vector can remain stale until another index run or backfill. | Requires overlapping re-illumination and indexing; normal new-capture flow enqueues indexing only after illumination persists. A dirty/version marker or deferred rerun would add coordination state for a narrow race.                                         | A stale vector is observed after re-illumination, or re-illumination becomes a common operation.      |
| **Attempt claiming is read-then-write** | Concurrent deliveries can both execute; racing reads can claim the same attempt number, while a later `InProgress` redelivery may increment it. | Cloud Tasks is at-least-once; there is no atomic claim or lease fencing workers. Duplicate work and status races are accepted for the prototype. | Concurrent duplicates cause noticeable cost, incorrect status, or user-visible result ambiguity. |
| **Status transitions are permissive** | A late attempt can overwrite a newer status or result reference because finish updates are keyed by run but not fenced by attempt or current status. | Duplicate overlap is possible, but strict transition/fencing logic adds lifecycle complexity; accepted for the prototype. | Out-of-order delivery produces visible incorrect status or affects retry behavior. |
| **Entity snapshot order is not a presentation contract**                     | The latest-run query orders rows by entity, task type, and descending run for PostgreSQL `DISTINCT ON`, not by UI presentation needs.                                                                                                      | Current consumers need the latest row per entity/task type, not a custom cross-entity order.                                                                                                                                                                    | The UI requires a different explicit ordering.                                                       |

> **Note on the removed guards:** the `illuminate` and `search_index` idempotency
> guards were deleted outright. A "skip if already illuminated" check silently
> no-ops every rerun (reporting success while doing no work), so keeping it meant
> deriving the run number from the illumination count — machinery that didn't
> reliably prevent the duplicate it existed for. Removing is simpler and makes
> reruns work by construction. A `UNIQUE(capture_id)` constraint on
> `illuminations` is *not* wanted: multiple illuminations per capture is the
> intended outcome of reruns.
>
> **Note on illumination visibility:** reruns append rows, so `InfoMaker` now
> collapses `illuminations` to the highest `id` — `CaptureInfo.illuminations` has
> at most one entry. This is enforced in one place rather than relying on loader
> order (SeaORM orders related rows by primary key *ascending*, so `.first()`
> would have returned the oldest).

### Cost / throughput

| Issue                                                                                                 | Effect                                                                                                                                | Why tolerated                                                                                | Revisit trigger                                                               |
| ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **`api/user/client.rs` swallows `Beacon.new_capture` errors** (logs a warning, still returns success) | A capture can be uploaded but never illuminated/indexed, with no user-visible signal.                                                 | The upload itself succeeded, which is what the user cares about. Failure is visible in logs. | A user notices a capture that never got illuminated.                          |
| **`CloudTaskQueue` URL configuration is deployment-specific**                                         | The same `TASK_WEBHOOK_BASE_URL` must point to a reachable local host or public production origin, depending on the selected backend. | The queue backend is mutually exclusive, and one setting avoids duplicate configuration.     | A deployment needs separate task targets or URL validation becomes important. |

### Security / defense-in-depth

| Issue                                                                          | Effect                                                                                                                                                                                                                             | Why tolerated                                                                                                                                                                       | Revisit trigger                                                                                       |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| **Task envelope fields are not cross-checked against the stored run identity** | A malformed or stale worker envelope can pair an existing `(envelope_id, run)` with a different `user_id`, task type, entity, or payload; execution and SSE metadata could then disagree with the original submission.             | Queue payloads are produced internally and the webhook has OIDC protection. Cross-checking/payload fingerprints would add task-framework work while we validate the app's use case. | Enable a second user, accept task envelopes from another producer, or observe a mismatched run/event. |
| **Worker update for a missing task-run row is warned and ignored**             | A stale or malformed delivery may execute without a matching persisted status row; no SSE status event is sent for its update.                                                                                                     | Queue envelopes are internal and authenticated; ignoring avoids retrying work whose tracking row is absent, while the warning keeps the inconsistency visible.                      | Observe a missing-row warning, or change stale-delivery policy.                                       |
| **Envelope `user_id` is not validated against the capture owner**              | `logic::spark::exec` derives `user_id` from the captures and never compares it to `envelope.user_id`. `api/service/get_capture.rs` is explicitly not user-scoped.                                                                  | The webhook routes are protected by Cloud Run OIDC, so an attacker can't reach them. This is defense-in-depth against our own bugs, not an open door.                               | Adding a second user, or any path where a webhook could be invoked with a mismatched envelope.        |
| **`TaskRunTracker::create_run` is check-then-act**                             | `submit_inner` reads the latest run, then inserts. Two concurrent submitters can both pick the same run number.                                                                                                                    | The `(envelope_id, run)` unique index arbitrates: the loser's insert fails and is surfaced as a refusal (`Ok(false)`), so correctness does not depend on the read.                  | If the refusal path ever gets logged as an unexpected error, or a second user is added.               |
| **Webhook OIDC verification depends on Google's JWKS endpoint**                | A cold instance or key rotation can add latency or temporarily fail webhook authentication if Google's certificate endpoint is unavailable. The Google crate caches each key for one hour, but does not retry a failed JWKS fetch. | `google-cloud-auth` provides maintained key fetching/caching; custom retry/backoff infrastructure is outside the MVP.                                                               | Authentication latency or task retries become operationally noticeable.                               |

### Operational

| Issue                                                 | Effect                                                                                                                                    | Why tolerated                                                                                            | Revisit trigger                                                                         |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| **No retention / vacuum policy on `task_run_status`** | The table grows forever. Note the run dimension means a reran task keeps *all* its rows, so growth is per-run rather than per-task.       | Tiny table, single user.                                                                                 | Row count becomes non-trivial, or query latency degrades.                               |
| **No heartbeat / timeout for stuck tasks**            | A task enqueued but never picked up (queue dropped, worker crash) stays `Queued` forever and looks active.                                | Requires an actual lost task, which hasn't happened.                                                     | First observed stuck task, or when the SSE UI makes it visible to the user.             |
| **No queue-level reconciliation**                     | The database cannot independently verify that a `Queued` row has a corresponding Cloud Task, or recover from an ambiguous enqueue result. | The app is small and the status row plus logs are sufficient operational visibility for the MVP.         | A task is observed stuck or missing, or Cloud Tasks/DB state needs auditing.            |
| **SSE heartbeat timeout is heuristic** | A delayed or suspended client can restart a healthy stream after 50 seconds without a heartbeat, adding a request and catch-up query. | The application heartbeat tells JavaScript that SSE events are still arriving; transport-level activity alone does not prove the browser received events. Background delivery remains best-effort. | Reconnects are observed on healthy delayed clients, or the 50-second detection window proves too long. |
| **Composite index on `task_run_status` deferred**     | The entity snapshot query filters by user, entity type, and IDs, then selects the latest run per task type; only a single-column `entity_id` index exists. | Single-user app, tiny table. | Query latency degrades, or the table grows past a few thousand rows. |
| **Local queue shutdown is abrupt**                    | Dropping the final `LocalTaskQueue` handle aborts its dispatcher and drops pending in-memory tasks.                                       | The local backend is only for development and tests; production durability comes from Cloud Tasks.       | Local development needs restart-safe work or graceful shutdown testing.                 |
| **`submit_inner` records status before enqueueing**   | A process crash or ambiguous backend failure can leave a row at `Queued`; definite enqueue failures are changed to `SubmissionFailed`.    | DB-first submission avoids untracked tasks and duplicate execution; reconciliation can wait for the MVP. | A task is observed stuck or missing, or enqueue ambiguity becomes operationally costly. |
| **No explicit Gemini inference request timeout** | A stalled Developer API or Vertex call can keep an illumination worker occupied until the outer request/task deadline. | Defer client timeout behavior while validating the product; rely on existing outer deadlines for now. | A hung Gemini request causes task retries, duplicate spend, or delayed illumination; then set and test a client timeout shorter than the worker deadline. |

### Code cleanliness

| Issue                                                                    | Effect                                                                                                                                                                                                                                                       | Why tolerated                                                                                                                                                                                                                                                                                                                                              | Revisit trigger                                                                                                             |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| **No atomic handoff between illumination and indexing**                  | A process crash after illumination persistence but before Beacon submits search indexing can leave a capture unindexed. A failed submission retries illumination, which can append another illumination.                                                     | An outbox/transactional handoff is beyond the prototype's current scope; retries usually recover and re-index.                                                                                                                                                                                                                                             | A capture is observed illuminated but still unindexed after task retries, or the handoff gap becomes operationally visible. |
| **"Two-owner rule" is slightly aspirational**                            | Docs say only `TaskMaster` (writes) and `StatusListener` (reads) touch `task_run_status`, but `TaskRunTracker` also touches it directly (shared by both).                                                                                                    | Harmless — the tracker is the shared persistence utility.                                                                                                                                                                                                                                                                                                  | If a third caller appears.                                                                                                  |
| **Task execution result return shape** | `logic::illuminate::exec` currently returns `(capture_user_id, TaskRunResultRef)`, so the caller receives output identity through a positional tuple. | Keep the current minimal shape until the shared `TaskEntity`/`TaskEntityRef` design is settled; avoid prematurely conflating system resource kinds with knowledge-graph entity categories. | When revisiting `TaskEntity`/`TaskEntityRef`, replace the tuple result with the shared `EntityRef` concept. |

---

## Deferred to dedicated sessions

These aren't "tolerated" so much as "explicitly out of scope for now, with a
plan to do them properly."

| Topic                        | Notes                                                                                                                                                                                                                                                                                                                       |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Backfill / bulk tasks**    | The `background` flag was removed (never populated). Backfill handling — marking tasks as bulk, surfacing progress, an admin view — plus the `user_id` mis-attribution and the global candidate query in `get_captures_need_search_index`, all get a dedicated plan-and-branch session. See `plans/task-status.md` §8.      |
| **Reruns**                   | The run *dimension* is implemented (2026-09-16): `run` column, `(envelope_id, run)` unique constraint, submit-time refusal for in-flight runs, run-scoped attempts, and both idempotency guards removed. Still deferred: a `model`/`force` field on `IlluminationTask` and a rerun endpoint. See `plans/task-status.md` §6. |
| **Capture lifecycle events** | Explicitly out of scope for the task-status phase; gets its own mechanism. See `plans/sse.md` §9.                                                                                                                                                                                                                           |

---

## Resolved (kept for context)

Fixed because they were cheap, unambiguous, and didn't distract from product
work. Recorded here so the ledger is complete.

| Issue                                                                                      | Resolution                                                                                                                                                     |
| ------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `task_run_status` had no entity linkage                                                    | Added `entity_type` + `entity_id` (indexed) and the incomplete-status query APIs.                                                                              |
| `TaskEnvelope` `Debug` could panic on a UTF-8 boundary                                     | Now takes a char-boundary-safe prefix.                                                                                                                         |
| Retry/exhaustion policy undefined (`attempts` hardcoded, `ErrorFinal` never written)       | Full policy: `task_max_attempts`, `ApiError::is_retryable()`, `AttemptOutcome`, and the ack-on-exhaustion HTTP mapping.                                        |
| `query_status` ignored `user_id`                                                           | Removed entirely (no callers).                                                                                                                                 |
| `background` always `false`                                                                | Field removed.                                                                                                                                                 |
| `ErrorWillRetry` returned `503`                                                            | Changed to `500` — `503` triggers Cloud Tasks *queue-wide* congestion throttling.                                                                              |
| `begin_attempt` could resurrect a completed task                                           | Now returns `None` for already-`CompleteSuccess` tasks, which the handler acks.                                                                                |
| Builder default `max_attempts` was 1                                                       | Now 3, mirroring the config default.                                                                                                                           |
| Assorted doc drift (`from_task` → `new`, stale "terminal" concept, `task_id` in examples)  | Corrected.                                                                                                                                                     |
| Postgres URL construction duplicated between `src/database` and `src/test_support/test_db` | Consolidated on `database::make_url_from_config`, which takes an optional schema. The harness now loads the app config instead of reading `DATABASE_URL`.      |
| `TaskRunTracker::record` was SELECT-then-INSERT, not an atomic upsert                      | Replaced by `create_run` (insert, unique-violation → `Ok(false)`) + `update_run` (targeted update). Duplicate submission is now *refused*, not a DB error.     |
| Duplicate task submission could queue the same work twice                                  | Submit now reads the latest run and refuses if it is in flight (`SubmitOutcome::RefusedInFlight`). `(envelope_id, run)` unique makes it race-safe.             |
| Reruns were indistinguishable from the original run                                        | Added the `run` dimension: a settled latest run permits a new run instead of overwriting the row.                                                              |
| `TaskStatusSnapshot` was a type that existed only to move two fields                       | Dropped; `query_status` returns the row (later split into `latest_run` / `query_run_status`), consistent with the other queries.                               |
| Stale-incomplete-run shadowing: a new completed run could be hidden by an older failed one | Incomplete queries collapse to the latest run *before* applying the incomplete predicate. Locked by the `completed_latest_run_hides_an_older_failed_run` test. |
| Idempotency guards in `logic/illuminate.rs` / `logic/search_index.rs`                      | **Deleted.** They were a cost optimization for duplicate work we tolerate, and the illuminate guard silently no-opped every rerun. Net LOC reduction.          |
| Reruns would have been served a stale illumination (`\| first` returned the *oldest*)      | `InfoMaker` collapses `illuminations` to the highest `id`, so `CaptureInfo.illuminations` is a single-element list by contract.                                |
| `google-cloud-aiplatform-v1` was used only for Vertex illumination | Removed; both Interactions backends now use the shared `reqwest` client and existing `google-cloud-auth`. The dependency is absent from `Cargo.toml` and `Cargo.lock`. |
