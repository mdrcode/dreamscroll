# Backfill management

> **Archived:** shipped design record. For current status see [plan/README.md](../README.md).

**Status:** Implemented: typed Backfill definitions, shared bounded pumping, TaskRun correlation, database uniqueness, Beacon descendant propagation, and admin API/CLI status reporting. The existing executor remains the execution path.

## Goal and scope

A small, code-defined backfill mechanism uses existing Tasks, TaskRuns, and TaskMaster queues. Creating or changing a Backfill requires a deployment. A stable string ID identifies one Backfill execution. Developers own the mapping from IDs to definitions; the application does not detect reuse of an ID after changing its definition. Change the literal ID when changing the workload or intentionally rerunning it, including recovery from failed runs.

Each definition implements the internal `Backfill` trait in `src/backfill/definition.rs`:

```rust
trait Backfill: Sync {
    type Task: task::Task;

    fn id(&self) -> &'static str;
    fn targets(&self) -> SelectStatement;
    fn make_task(&self, target: BackfillTarget) -> Self::Task;
}
```

`targets()` owns the candidate relation: it can select from any table and use joins or filters, but must return an **unpaginated** relation with one row per target and `i32` columns aliased as `entity_id` and `owner_id`. `TargetColumn` supplies those aliases. The associated Task supplies the static entity/task types. `make_task` receives the selected owner and ID and must preserve that identity in the complete concrete Task; execution parameters belong entirely to the implementation.

The shared engine wraps the candidate relation, excludes previously recorded work, orders by descending entity ID and owner ID, and then limits the batch. Definitions do not implement pumping or progress filtering. The engine does not materialize the unbounded candidate set.

To add a workload, implement `Backfill` in a dedicated module, declare it in the lean `src/backfill/mod.rs` root, and add its ID/variant and typed TaskMaster submission adapter to the registry in `src/backfill/engine.rs`. The registry supports heterogeneous concrete Task types without boxed Tasks, runtime task-type routing, or changes to TaskMaster. Core operations and schema types live under `crate::backfill`; AdminApiClient remains the admin-authorization boundary, and REST/CLI callers reference backfill types directly rather than API re-exports.

Do not add a generic job framework, a second task execution system, or a per-target BackfillItem table.

## Existing framework and ownership

The current contracts in `src/task` are authoritative:

- `Task` supplies static `task_type()` and `entity_type()`, plus instance `entity_id()` and `entity_owner_id()`. Concrete Tasks carry ownership and execution inputs together.
- `TaskRun<T>` contains `logical_id`, `run_id`, `run_number`, `task`, and optional `backfill_id` metadata. It does not duplicate the Task's target identity or contain a result-entity reference.
- `TaskRunTracker` derives the persisted flat `(entity_owner_id, entity_type, entity_id)` columns from the Task and stores its full serialized payload in `task_payload`.
- `TaskMaster` owns typed queues for illumination, search indexing, and spark. Public `submit_*` methods authorize the Context: the caller must be the Task's owner or an admin. Trusted worker follow-ups have an internal submission path.
- TaskMaster creates the `Queued` row before enqueueing. A queue error changes it to `SubmissionFailed` and is returned to the caller. A refusal creates no row.

The backfill factory must set the Task's owner from the server-side target query, never the administrator's Context. Do not add separate owner/type/ID submission arguments or restore a TaskEntity wrapper. Existing persistence, logical identity, owner-scoped status queries, and SSE routing use that Task ownership.

The current executor trusts admin/internal producers to supply the correct owner; it does not generally cross-check Task ownership against the persisted target or validate the worker payload against the stored run. Do not describe worker ownership verification as implemented or make adding it a backfill prerequisite. This trust boundary is recorded in `plan/pragmatism.md`.

Keep existing execution behavior. For example, successful v2 illumination invokes Beacon to submit search indexing with the parent Task's owner; v1 illumination is raw-only. Backfill uses these same execution paths, extending worker-to-Beacon submission only to propagate invocation correlation metadata.

## TaskRun correlation and in-flight behavior

`TaskRun<T>` and `task_runs` carry optional `backfill_id`. Ordinary runs leave it empty; directly pumped runs and their descendants carry the stable Backfill ID. It is invocation metadata, not part of the concrete Task's execution payload. It is set before creating the row and enqueueing so the database and worker envelope agree; retries retain it with the same `run_id`.

Typed TaskMaster submission methods accept a final `backfill_id: Option<&str>` argument without changing authorization or task-specific validation. Ordinary callers pass `None`. Queue selection stays typed and `submit_inner` stays private; the pump does not insert TaskRuns or enqueue directly.

Do not include `backfill_id`, execution parameters, or run identity in `logical_id`. Its current format is `u{owner}-{task_type}-{entity_type}{entity_id}`. Live work, different backfills, and different model/prompt choices for that same logical task share the existing in-flight guard. A new invocation gets a fresh UUID `run_id` and the next `run_number`; retries are attempts within that invocation.

- `Queued`, `InProgress`, and `ErrorWillRetry` block new submissions. `SubmissionFailed`, `CompleteSuccess`, and `CompleteFailure` permit a new run under normal TaskMaster policy.
- If live work is in flight, backfill submission is refused without a backfill row; that target remains eligible for a later pump. If backfill work is in flight, a live submission is likewise refused.
- The existing unique `(logical_id, run_number)` constraint arbitrates concurrent claims for the same next run. It does not prevent another run for a backfill target after the first run settles; backfill-level deduplication is a separate concern.

A unique database index on `(backfill_id, logical_id)` enforces backfill deduplication. `logical_id` already includes target owner, task type, entity type, and entity ID; `run_id`, `run_number`, and status are excluded. For a non-null backfill ID, at most one invocation of each logical task belongs to that backfill, even after it settles. PostgreSQL's default NULL-distinct uniqueness leaves ordinary runs (`backfill_id IS NULL`) free to rerun. The existing `(logical_id, run_number)` constraint still arbitrates live/backfill in-flight submissions.

This constraint is the backfill deduplication boundary for both pumped and descendant tasks; whole-pump serialization is unnecessary. Concurrent pumps may select the same target, but only one can persist and enqueue the corresponding backfill run. The loser is an expected already-submitted outcome, not an error and not necessarily “in flight.” Preserve that distinction in submission/pump reporting. Classify the known unique conflicts rather than treating every unique violation as an in-flight refusal. A repeated descendant submission already represented by a backfill row must likewise not fail its parent merely because it was deduplicated.

Worker handlers pass the parent TaskRun's optional `backfill_id` through Beacon and trusted typed follow-up submission. Every descendant that is actually created inherits it; it is not inferred from the entity or the latest run. A normal parent produces untagged descendants. No parent-run column or dependency graph is needed for aggregate reporting.

The uniqueness key includes task type through `logical_id`, so an illumination and its search-index descendant can both belong to the backfill. Multiple parents producing the same logical child within one backfill share one child invocation, rather than counting submission attempts as fan-out.

Do not retag an existing live run or another backfill's run when a descendant submission is refused by the shared in-flight guard. Such a refusal creates no descendant row for this backfill. Counts describe actual tagged invocations, not all expected workflow edges: they cannot prove every parent produced a successful descendant. Keep this limitation explicit rather than adding dependency tracking or deferred follow-up scheduling.

## Target selection and pumping

The target relation is queried, not materialized as an unbounded collection. Conceptually:

```text
SELECT target.*
FROM (<Backfill's code-defined target query>) AS target
WHERE NOT EXISTS (
  SELECT runs.id FROM task_runs AS runs
  WHERE runs.backfill_id = <backfill_id>
    AND runs.entity_owner_id = target.owner_id
    AND runs.task_type = <backfill_task_type>
    AND runs.entity_type = <Backfill::Task::entity_type()>
    AND runs.entity_id = target.entity_id
)
-- Deliberately do not skip live in-flight work: short-lived contention may
-- fill a batch and temporarily starve lower IDs. A later manual pump retries.
ORDER BY target.entity_id DESC, target.owner_id DESC
LIMIT <BATCH>
```

The implementation composes SeaORM/SeaQuery builders: an outer query selects from the definition's normalized relation and applies a correlated `NOT EXISTS` subquery **before** the shared limit. No raw SQL statement is embedded. Status aggregation likewise uses typed selection, `COUNT(id)`, grouping, and ordering methods. PostgreSQL counts are decoded as `i64`; the non-null row primary key makes `COUNT(id)` equivalent to counting rows.

Each manual pump performs one bounded query, creates a complete Task for each selected target, and submits it through the matching typed TaskMaster path with backfill metadata. Do not load the entire target set or loop until exhaustion inside one API request. The batch size limits submission work per pump, not the total queue backlog; execution concurrency and dispatch rate remain the existing queue backend's responsibility.

TaskRun rows are the per-target progress markers; no persistent cursor is planned. Repeated pumps select targets without a matching backfill run. An existing live run, or a run from another backfill, is not a progress marker for this backfill.

The query is evaluated against current data on every pump; membership is not frozen. A zero-row result means no currently eligible targets lack a backfill TaskRun. It does **not** mean submitted work has finished or succeeded. Retain the tagged TaskRun rows for as long as the backfill ID may be pumped; deleting them removes its progress markers.

**Accepted selection tradeoff:** an entire descending-ID batch can be occupied by targets refused because live work is in flight, temporarily starving lower IDs. Live work is expected to be short-lived and isolated, so keep the simple query and pump again later. Do not add in-flight filtering, scan-ahead, or a cursor to avoid this. Revisit if contention or stuck live runs cause persistent lack of progress. Preserve this explanation in the implemented selection query's comments; the sketch above includes it.

## First definition and operation

`src/backfill/illuminate_v2_backfill.rs` implements `IlluminateV2Backfill` with ID `illuminate-v2-all-captures-2026-10-16`: **every capture**, including archived and already illuminated captures, using `gemini-3.8-flash` and illumination v2. It is registered in `src/backfill/engine.rs`. Each request selects up to its candidate limit (**100 by default**), descending by capture ID. V2's normal Beacon path subsequently submits search indexing with the same backfill ID. The model is part of this code-defined implementation, not read from mutable environment configuration at pump time. Changing the per-pump limit does not change the workload or its ID.

```sh
dreamscroll_api --host localhost:8080 backfill pump --limit 25 illuminate-v2-all-captures-2026-10-16
dreamscroll_api --host localhost:8080 backfill status illuminate-v2-all-captures-2026-10-16
```

Repeat pumps manually until `selected_count` is zero; inspect status separately for queued work or failures. Pump CLI output is explicitly labeled **this invocation only**: candidates attempted, newly enqueued, skipped because already recorded for this backfill, skipped because a conflicting run is in flight, and submission errors. These are submission outcomes, not cumulative backfill status; the output points to `backfill status` for overall progress.

`pump --limit N` optionally sets the maximum candidates attempted by that invocation. Omission uses 100; values above 100 are supported. The limit must be positive (zero returns HTTP 400). It caps attempts, not successful enqueues: refusals/errors can reduce the number enqueued, and the pump does not scan ahead to fill those slots. Descendant tasks do not count against this root-candidate limit. Progress exclusion still happens before limiting.

Both operations require an admin account. Unknown IDs return 404. REST endpoints are `POST /api/admin/backfill/pump` with `{"backfill_id":"<id>","limit":25}` (the `limit` field is optional) and `GET /api/admin/backfill/<id>/status`. The old ad-hoc search-index backfill command and enqueue endpoint have been removed.

## Status and failures

Use all TaskRuns tagged with `backfill_id` for per-target and aggregate status, not the existing owner-scoped latest-run queries: a later live run must not hide the backfill's run, and an administrator needs counts across target owners. Add admin-authorized backfill queries rather than weakening the user query scope.

Any associated backfill TaskRun, including `SubmissionFailed` or `CompleteFailure`, excludes that target from later pump queries. This is a submission/progress marker, not a success marker. The backfill layer does not create replacement runs for failures; existing within-run retry policy still applies. In production, TaskMaster records `ErrorWillRetry` and the worker returns HTTP 500 for Cloud Tasks redelivery; success and terminal execution failure are acknowledged with 2xx. The local queue does not retry failed delivery, so it is not equivalent to production retry behavior.

Report counts grouped by `task_type` and `status_code`, filtered by `backfill_id`, with per-target run IDs available for inspection. This includes directly pumped tasks and all tagged descendants and exposes the aggregate fan-out without a separate item table. Count TaskRun rows, not attempts; retries update the same row. Keep “no targets left to submit,” “recorded runs still in flight,” and “all recorded runs succeeded” distinct. Counts grow as descendants are submitted; with live membership and no expected-descendant ledger there is no frozen total or durable workflow-completion state.

Each pump should report selected, enqueued, already-submitted-for-this-backfill, refused-in-flight, and errored submissions. An error is not proof that no row exists: enqueue failure normally leaves a tagged `SubmissionFailed` row, and failures after persistence can leave a row even when the request returns an error. Persisted TaskRuns are authoritative across partial pump failures and repeated requests. Do not add queue reconciliation or a second retry policy here; retain the existing DB-first submission tradeoff.

Concurrent pumps are allowed. The `(backfill_id, logical_id)` unique constraint prevents duplicate backfill invocations even when the first run finishes before another pump submits. The target query is an optimization for selecting unsubmitted work; the database constraint is authoritative.

`database::indexes::task_run` manages the unique `(backfill_id, logical_id)` index and a `(backfill_id, task_type, status_code)` reporting index during explicit schema sync and in the isolated test harness. Explicit index setup is needed because the current SeaORM derive cannot express overlapping composite unique keys on `logical_id`. Query-plan tuning for larger datasets can be revisited without changing the progress model.

## Implementation and verification

- Correlation and deduplication: `src/task`, `src/model/task_run.rs`, and `src/database/indexes/task_run.rs`.
- Definition contract and normalized target schema: `src/backfill/definition.rs`; the first workload: `src/backfill/illuminate_v2_backfill.rs`; generic selection, pumping, registry dispatch, and status: `src/backfill/engine.rs`; shared request/result types: `src/backfill/types.rs`. `src/backfill/mod.rs` only declares/re-exports modules and tests. Behavior and extensibility tests live alongside the implementation in `src/backfill/tests.rs` and `src/backfill/extension_tests.rs`.
- Descendant propagation: the illumination webhook and Beacon. Existing v1/v2 execution and retry behavior are unchanged.
- Admin authorization, REST client/routes, and CLI: the existing admin, REST, and `util_cmds/api/backfill.rs` modules.

All 239 all-target tests passed against local PostgreSQL; doctests reported one passed and four ignored. The extensibility regression uses a filtered join over sparks, creates concrete SparkTasks through the same generic pump, verifies exclusion before batching, and keeps other task types/backfill IDs independent. Existing correlation, failure, ownership, descendant, and concurrency regressions still pass.

A disposable-schema runtime smoke exercised the registered trait implementation and actual local queues: 101 captures were pumped in batches of 100, 1, and 0; aggregate status reported 101 queued illuminations, with v2 payload parameters and capture ownership preserved. No external inference or Cloud Tasks calls were made. Temporary smoke code and its schema were removed.

After the module-only relocation, `cargo check --all-targets` and `dreamscroll_api backfill --help` passed. Unit tests were not rerun for this namespace/plumbing change; the preceding full-suite and database smoke results describe the unchanged implementation.

For the optional candidate-limit extension, all 15 backfill regressions passed against PostgreSQL, including limits below/above the default, refused attempts consuming candidate slots, and zero rejection without consuming targets. CLI help exposes `--limit`. A disposable-schema runtime smoke deserialized real JSON requests with explicit and omitted limits and observed batch counts 2, 1, 100, 2, and 0; zero was rejected and 105 total runs were persisted. No external inference or Cloud Tasks calls were made; temporary smoke code and schema were removed.


## Failure detail: `backfill failures`

**Added 2026-10-24.** Aggregate `status` counts answer "how many failed"; they do
not say which targets or why. This adds the per-target view.

- **Engine:** `backfill::failures(db, backfill_id)` selects runs tagged with the
  backfill whose status is terminal and unsuccessful — `CompleteFailure` (a worker
  ran it and exhausted its retries) or `SubmissionFailed` (it never reached a
  worker) — ordered by entity ID then task type.
- **No collapse step.** The `(backfill_id, logical_id)` unique index means a
  tagged target has exactly one tagged run, so no later run can supersede a
  failure and there is no "latest run wins" reconciliation to perform.
- **What each row carries:** a `TaskRunInfo` — the same projection the per-run
  status endpoints return — so it includes `logical_id`, `run_id`, `run_number`,
  `task_type`, `entity_type`/`entity_id`, `status`, `attempts`, `created_at`,
  `updated_at`, and the error fields described in `plan/archive/task-status.md`
  §10. A `CompleteFailure` carries the worker's error; a `SubmissionFailed`
  carries the enqueue error written at submission time, so both are triageable
  without the pump response or Cloud Logging. Rows written before schema version
  7 have `NULL` there. No backfill-specific type is needed: the row is already a
  serializable task-run status with its errors.
- **"Terminal" means the last reporting layer said so.** A run abandoned by the
  delivery pipeline — worker killed mid-attempt (`in_progress`) or no further
  retry delivered (`error_will_retry`) — never advances and is not listed here;
  it appears only in `status`, which counts in-flight work. Excluding those
  statuses is deliberate: including them would report healthy retries as
  failures.
- **Descendants included**, since they inherit `backfill_id`.
- **No pagination.** The result is bounded by the number of failed targets, not
  runs, and the table is small; a limit would silently hide failures from a
  command whose purpose is enumerating them.

Surfaces: `GET /api/admin/backfill/<id>/failures` (admin only, `404` for an
unknown ID), `AdminApiClient::backfill_failures`,
`rest::client::Client::admin_backfill_failures`,
and `dreamscroll_api backfill failures <id>`. The CLI prints one line per failure
with the entity, task type, run/attempt counts, status, and error; embedded
newlines in a persisted message are collapsed so each failure stays on one line.

Verification: `failures_report_terminal_runs_with_the_error_each_recorded`
asserts against PostgreSQL that a `CompleteFailure` with a stored 403/message
appears while a success and an in-flight run do not.
`a_partial_pump_failure_is_reported_and_not_automatically_resubmitted` creates a
real `SubmissionFailed` run through a rejecting queue and asserts it is listed
with the enqueue error (`500`, `queue rejected capture 2`). The admin/unknown-ID
test covers `403` and `404`.

Live smoke (2026-10-25) against a local dev server on the persistent dev database:
`GET /api/admin/backfill/illuminate-v2-all-captures-2026-10-16/failures` returned
`200` with all 17 pre-existing terminal failures (captures 88–672); non-admin
token `403`, unknown ID `404`, no token `401`. The CLI printed the same 17 rows as
`- capture <id> [illuminate run 1, 1 attempts, complete_failure]: no error recorded`
— those runs predate schema version 7, so
both error columns are `NULL`. The `code + message` rendering is covered by the
`describe_error` unit test rather than the smoke, which would have required
writing error values into the dev database.
