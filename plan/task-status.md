# Task Status — Design & Implementation

**Status:** **Implemented and merged.** This documents the task framework as it
exists on this branch.
**Scope:** Task identity, queueing, status persistence, the retry/exhaustion
policy, the run dimension, and the query API.

> **See also:**
> - `sse.md` — the future SSE delivery layer that consumes this (not implemented).
> - `pragmatism.md` — the ledger of deliberately tolerated trade-offs.
> - `testing.md` — the two-tier test model (unit vs. DB).

---

## 1. Problem statement

When a user uploads a screenshot, the app enqueues an AI-powered illumination in
the background. There was **no record of that work** — no way to know whether it
was queued, running, succeeded, or failed, and no way to retry it deliberately.

This document covers the framework that fixes that: a first-class `task_run_status`
table, a typed task identity, a retry/exhaustion policy, and a run dimension that
makes reruns expressible.

> **Scope boundary:** this is about **task state only**. Relaying that state to
> the browser (SSE) is a separate, future project — see `sse.md`. Capture
> lifecycle events (a capture uploaded/deleted on another device) are a separate,
> TBD concern.

---

## 2. Architecture

### 2.1 The upload → illumination flow

```
Upload (webui/v2/r_upload.rs)
  └─ insert_capture() → task_master.submit_illumination(user_id, IlluminationTask)
       └─ /_wh/cloudtask/illuminate → logic/illuminate::exec
            ├─ illuminate_capture()        (no idempotency guard — see §7)
            └─ logic/search_index::exec    (no idempotency guard — see §7)
                 └─ insert_illumination()  ← row written, status now tracked
```

> **Note:** `IngestTask` and `logic/ingest.rs` were **removed**. Illumination has
> no real purpose without search indexing, so `logic/illuminate::exec` now runs
> **both** steps as a single unit of work. The capture-create path calls
> `submit_illumination` directly. `r_illuminate` + the illumination queue are the
> live path; the `/_wh/cloudtask/illuminate` route is reserved for future
> backfill / rerun flows.

### 2.2 Key facts

- **`Task` is a trait, not an enum.** Each concrete task type
  (`IlluminationTask`, `SparkTask`, `SearchIndexTask`) lives in `src/logic/*.rs`
  and implements `task::Task`. The trait carries the task's **identity**:
  `task_type() -> &'static str`, `entity_type() -> &'static str` (e.g.
  `"capture"`, `"spark"`), and `entity_id(&self) -> i32`. A `TaskEnvelope<T>`
  wraps a task with `user_id`, `envelope_id`, `run`, and the payload
  (`task: T`).
- **Task identity is deterministic.** `TaskEnvelope::make_envelope_id(user_id,
  task)` builds `envelope_id = "u{user_id}-{task_type}-{entity_type}{entity_id}"`
  (e.g. `u1-illuminate-capture123`). There is **no UUID** and no separate
  `task_id.rs`. The id names the *logical work* and deliberately **excludes** the
  run — see §6.
- **`TaskQueue<T>` is enqueue-only and generic.** Status lives in the
  `task_run_status` table. The trait is `async fn enqueue(&self, wrapped:
  TaskEnvelope<T>)`. Two backends: `LocalTaskQueue` (in-process mpsc +
  semaphore) and `CloudTaskQueue` (Google Cloud Tasks). **Pub/Sub support was
  removed** to focus on Cloud Tasks.
- **`TaskMaster`** (`task/taskmaster.rs`) is the single public funnel through
  which *all* task enqueues and worker lifecycle updates flow. It owns the
  backend queues and coordinates lifecycle policy and queue behavior. It exposes
  `submit_*` / `begin_attempt` / `finish_attempt` / status queries. It is **not
  `Clone`** — shared via `Arc<TaskMaster>`.
- **Status transitions are not a raw setter.** `TaskMaster::update_status` is
  **private**; workers must go through `begin_attempt` (reads the persisted
  attempt count, increments, writes `InProgress`, returns the 1-based attempt
  number) and `finish_attempt` (writes the outcome and returns a status that
  drives the HTTP response). This keeps `attempts` and the
  retry decision consistent with the recorded status.
- **`TaskRunTracker`** (`task/taskruntracker.rs`) is a private persistence
  component owned by `TaskMaster`; Rust visibility prevents production callers
  outside the `task` module from bypassing TaskMaster's lifecycle API. It owns
  direct `task_run_status` queries/writes and emits an optional best-effort
  status event after successful inserts and updates. Its notifier is injected
  through the `ServerEventNotifier` trait; notification failures are logged and
  do not fail persistence. The task composition factory selects the PostgreSQL
  implementation and injects it through `TaskMasterBuilder`; tests may inject a
  recorder or omit notifications.
  `TaskRunStatus`
  (`task/taskrunstatus.rs`) is the strongly-typed status enum; the DB stores
  only its integer discriminant (`status_code INT`).
- **Deployment is a single Cloud Run service.** Tasks are queued via Cloud Tasks,
  so **the worker that completes a task may be a different process/instance than
  the one holding the user's HTTP connection.** This is why the DB — not an
  in-process bus — is the source of truth (see `sse.md` §4.1).

---

## 3. The `task_run_status` table

The small, focused **`task_run_status` table** is the source of truth:

```sql
CREATE TABLE task_run_status (
    id            BIGSERIAL PRIMARY KEY,
    user_id       INT NOT NULL,
    envelope_id   TEXT NOT NULL,          -- logical task identity (§2.2)
    run           INT NOT NULL,           -- which run of the logical task, from 1
    task_type     TEXT NOT NULL,          -- 'illumination' | 'spark' | 'search_index'
    entity_type   TEXT NOT NULL,          -- 'capture' | 'spark'
    entity_id     INT NOT NULL,           -- the entity this task operates on
    status_code   INT NOT NULL,           -- integer discriminant of task::TaskRunStatus
    attempts      INT NOT NULL DEFAULT 0, -- 1-based attempt number within this run
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (envelope_id, run)
);
```

> **Schema management:** the table is defined by the SeaORM model
> (`src/model/task_run_status.rs`) and created/synced automatically at startup via
> `conn.get_schema_registry("dreamscroll::model::*").sync(&conn)` in
> `database/postgres.rs`. There is no hand-written migration file.

> **Why `UNIQUE (envelope_id, run)` and not `UNIQUE (envelope_id)`:** one row per
> *run* is what makes reruns expressible. `envelope_id` alone identifies the
> logical task; adding `run` lets a rerun append a new row instead of overwriting
> the previous outcome. The pair is also the guard that makes duplicate
> submission safe — see §6.

- **`status_code`** stores the integer discriminant of `task::TaskRunStatus`
  (`SubmissionFailed=0`, `Queued=1`, `InProgress=2`, `ErrorWillRetry=3`,
  `CompleteSuccess=4`, `CompleteFailure=5`). **These integers are persisted —
  do not renumber them.**
- **`entity_type`/`entity_id`** are the queryable entity linkage. They are what
  makes "give me all the incomplete task statuses for capture 123" expressible
  (§5).

> **Why a focused `task_run_status` table (not a generic `events` table):** task state
> is a first-class, non-trivial problem of its own — it has a lifecycle, retries,
> and reruns. A dedicated table with typed columns models that cleanly and is
> queryable. A generic `kind`+`payload` JSONB table would dilute this and make
> task-state queries awkward.

This also fixes a latent bug from the audit notes: *"No task retry/dead-letter in
LocalTaskQueue — failed tasks silently dropped."* With a `task_run_status` table,
`ErrorWillRetry`/`CompleteFailure` states become observable.

---

## 4. The status vocabulary and retry policy

**The status vocabulary** (`task::TaskRunStatus`, `src/task/taskrunstatus.rs`):

| Variant            | Code | Meaning                                                                 |
| ------------------ | ---- | ----------------------------------------------------------------------- |
| `SubmissionFailed` | 0    | Queue submission failed before a worker could receive the task.         |
| `Queued`           | 1    | Enqueued, not yet picked up.                                            |
| `InProgress`       | 2    | A worker is currently executing an attempt.                             |
| `ErrorWillRetry`   | 3    | Failed, but the app still has retry budget — another attempt is coming. |
| `CompleteSuccess`  | 4    | Succeeded.                                                              |
| `CompleteFailure`  | 5    | Failed permanently (budget spent, or the error was non-retryable).      |

> **`ErrorWillRetry`/`CompleteFailure` are *computed outcomes*, not intrinsic
> properties of an error.** The same underlying failure is `ErrorWillRetry` on
> attempt 1 and `CompleteFailure` on the final attempt. The decision is made by
> `AttemptOutcome::from_failure(err, attempt, max_attempts)`.

### 4.1 Two predicates, deliberately distinct

`TaskRunStatus` exposes the predicates used to distinguish worker eligibility
and user-visible incomplete work:

| Predicate         | Members                                  | Question it answers                 |
| ----------------- | ---------------------------------------- | ----------------------------------- |
| `is_in_flight()`  | `Queued`, `InProgress`, `ErrorWillRetry` | May a worker still act on this?     |
| `is_incomplete()` | the above **+ `CompleteFailure`**        | Does the user still need to see it? |

`CompleteFailure` is the **only** execution status where they disagree: the user still needs
to see the failure, but no worker will touch it again — which is exactly what
makes it rerunnable. A settled run (rerunnable) is simply `!is_in_flight()`;
there is deliberately no separate `is_settled()` predicate.

> **Do not conflate these.** `is_in_flight` gates duplicate-submission refusal
> (§6); `is_incomplete` gates the query API (§5). A test
> (`in_flight_is_not_the_same_as_incomplete`) locks the divergence.

### 4.2 The retry/exhaustion policy

- **`Config.task_max_attempts`** (env `TASK_MAX_ATTEMPTS`, default `3`) is the
  app's own retry budget. Threaded into `TaskMasterBuilder::max_attempts`.
- **`ApiError::is_retryable()`** classifies failures: 5xx (server errors) are
  transient and worth retrying; 4xx (client errors) are permanent — retrying
  identical input produces identical results.
- **`TaskMaster::begin_attempt(envelope)`** reads the persisted attempt count,
  increments it, writes `InProgress`, and returns the 1-based attempt number.
  Deriving the count from the DB (rather than Cloud Tasks' retry-count header)
  means it works identically for **every** backend, including `LocalTaskQueue`,
  which has no headers. It returns `None` when the run is already `CompleteSuccess`, so
  an at-least-once redelivery of successful work is acked without resurrecting the
  row to `InProgress`.
- **`TaskMaster::finish_attempt(envelope, attempt, &result)`** writes the outcome
  and returns an `AttemptOutcome`.

**The key convention: the Cloud Tasks queue is always configured with MORE max
retries than the app.** This means the app always exhausts its budget *first*, so
it can ack the task and stop Cloud Tasks from spending its remaining retries. The
HTTP mapping (`webhook::http_status_for_task_run`) follows from that:

| Outcome           | HTTP                        | Cloud Tasks behavior |
| ----------------- | --------------------------- | -------------------- |
| `CompleteSuccess` | `204 No Content`            | ack (stop)           |
| `CompleteFailure` | `200 OK`                    | ack (stop)           |
| `ErrorWillRetry`  | `500 Internal Server Error` | retry                |

> **Why `CompleteFailure` returns 2xx:** Cloud Tasks retries on *any* non-2xx and
> stops on *any* 2xx — there is no "fail but don't retry" status code. Since the
> app's budget is smaller than the queue's, the app must ack to short-circuit the
> queue's remaining retries.

> **Why `ErrorWillRetry` uses `500`, not `503`:** Cloud Tasks treats `503` (and
> `429`) as *system* errors and responds by throttling the **whole queue's**
> dispatch rate. That is a queue-wide side effect we don't want from an ordinary
> per-task failure, so we use `500`.

> **The 204-vs-200 distinction is a debugging nicety, visible only in Cloud Run
> request logs.** Cloud Tasks' own `lastAttempt.responseStatus` normalizes every
> 2xx to `OK`. The `task_run_status` row remains the source of truth.

> **Local dev:** `LocalTaskQueue` does not retry at all (it logs and drops on
> handler error), so `config_local.env` sets `TASK_MAX_ATTEMPTS=1` — a local
> failure lands on `CompleteFailure` immediately rather than appearing stuck at
> `ErrorWillRetry`.

---

## 5. The query API

The query API is deliberately **"incomplete", not "non-terminal"**. Two
entity-scoped entry points, both returning every row whose status is **not
`CompleteSuccess`**:

- `TaskRunTracker::query_incomplete_for_entity(user_id, entity_type, entity_id)`
  — the tasks for one entity (e.g. one capture).
- `TaskRunTracker::query_incomplete_for_user(user_id)` — every outstanding
  task for a user, across all entities.

Rationale:

- **`CompleteFailure` is included on purpose.** The work never succeeded, so the
  user still wants to see it (and may want to retry it). It is *not* "done".
- **`CompleteSuccess` is excluded on purpose.** Successful rows are subject to vacuuming
  over time, so an API that returned them would silently present an incomplete
  history. The API therefore cannot express "give me everything" — the usage
  pattern is enforced by what's available.
- The predicate is derived from `TaskRunStatus::is_incomplete()` via
  `TaskRunStatus::incomplete_codes()`, so the status set and the SQL predicate can
  never drift apart.

### 5.1 Only the latest run is returned

Both queries collapse to the **latest run per logical task** — a rerun supersedes
the run before it, and callers want current state, not a run history.

**The order matters:** rows are collapsed to the latest run **before** the
incomplete predicate is applied. Filtering first would let an older incomplete
run shadow a newer `CompleteSuccess` one, reporting finished work as outstanding. This is
locked by the `completed_latest_run_hides_an_older_failed_run` test.

Implemented by `incomplete_latest_runs()` in Rust rather than SQL: the result set
is per-user (or per-entity), which is small, and this avoids a correlated
subquery or window function.

> **TODO(REVISIT) — index.** The primary read patterns are `WHERE user_id = ? AND
> entity_type = ? AND entity_id = ?` and `WHERE user_id = ?`, which currently
> only have the single-column `entity_id` index. A composite index on
> `(user_id, entity_type, entity_id, status_code)` is the right long-term shape.
> **Deferred deliberately** — single-user app, tiny table. Note SeaORM's derive
> only supports single-column `#[sea_orm(indexed)]` and composite `unique_key`,
> so a non-unique composite index needs raw SQL.

---

## 6. Runs and reruns

`TaskEnvelope.envelope_id` names the **logical task**
(`u1-illuminate-capture123`) and `TaskEnvelope.run` names **one attempt to carry
it out**, counting from 1. `(envelope_id, run)` is unique and keys a `task_run_status`
row, so a rerun appends a row rather than overwriting the previous outcome.

A submission is planned by reading the latest run of the logical task
(`plan_submission`):

| Latest run                                                       | Decision                                           |
| ---------------------------------------------------------------- | -------------------------------------------------- |
| none                                                             | start run 1                                        |
| in flight (`Queued`/`InProgress`/`ErrorWillRetry`)               | **refuse** — the work is already queued or running |
| settled (`SubmissionFailed`/`CompleteSuccess`/`CompleteFailure`) | start `run + 1`                                    |

This gives two properties at once:

- **Duplicate submission is prevented.** Re-submitting work that is in flight is
  refused, so a double-clicked upload cannot queue the task twice.
- **Reruns work.** Once a run settles, the next submission starts a new run — so
  "illuminate this again" needs no special machinery, only a settled prior run.

The refusal is returned as `SubmitOutcome::RefusedInFlight { run }`, a normal
outcome rather than an `Err`: duplicates are expected, so callers should not log
them as failures. `SubmitOutcome::Enqueued { run }` reports the run that started.

> **Correctness rests on the unique index, not the read.** `plan_submission` is
> check-then-act: two concurrent submitters can both read "no prior run" and both
> try to insert run 1. The `(envelope_id, run)` unique index arbitrates — the
> loser's insert is a unique violation, which `create_run` maps to `Ok(false)`
> and `submit_inner` turns into `RefusedInFlight`. This is the same TOCTOU
> pattern tolerated elsewhere (`pragmatism.md`), except here the constraint makes
> it safe rather than merely unlikely.

Attempt counting is **per run**: each row carries its own `attempts`, so a rerun
starts at attempt 1.

### 6.1 What's still deferred (the rerun UX)

The run *dimension* is complete. What remains is the UX to trigger a rerun:

- A `model`/`force` field on `IlluminationTask` so a rerun can differ from the
  original (new model, new prompt).
- A rerun endpoint + button. The handler calls
  `task_master.submit_illumination(user_id, IlluminationTask { .. })`; the run
  logic starts the next run automatically once the prior one has settled.

```html
<button hx-post="/detail/{{ capture.id }}/rerun"
        hx-vals='{"model": "gemini-2.5-pro"}'>
  Re-illuminate
</button>
```

> **No way to reject a rerun after completion.** Because a settled run always
> permits a new one, a genuine duplicate submitted after completion is
> indistinguishable from an intentional rerun. Accepted: re-running is cheap and
> idempotent. Forcing a rerun *while one is in flight* would need an explicit
> `force` flag — deferred.

---

## 7. Idempotency: deliberately absent

**Both idempotency guards were removed** (`logic/illuminate.rs` and
`logic/search_index.rs`). `exec` is honestly at-least-once: every attempt
re-illuminates and re-embeds.

**Why they were removed:**

- They were a cost optimization for duplicate work we have **already declared
  tolerable** (`pragmatism.md`).
- The illuminate guard **silently no-opped every rerun** — a "skip if already
  illuminated" check reports success while doing no work. Making it correct meant
  deriving the run number from the illumination count, machinery that didn't
  reliably prevent the duplicate it existed for.
- Removing is simpler and makes reruns work by construction.

**Cost accepted:** a retry re-calls the LLM and re-embeds. Failing tasks are the
minority, and both are API calls we already tolerate duplicating.

> **Consequence:** `task_run_status.attempts` and the illumination count can diverge
> (a run that exhausts *after* inserting an illumination leaves a row behind).
> Harmless, since only the most recent illumination is displayed (§7.1).

### 7.1 Illumination visibility (append, don't replace)

Reruns **append** `illuminations` rows rather than replacing — the schema already
allowed multiple rows per capture, and keeping the history is useful. The user
must always see the **most recent** illumination, so:

- `InfoMaker::make_capture_info` collapses `illuminations` to the single row with
  the highest `id`. `CaptureInfo.illuminations` therefore has **at most one**
  entry, by contract.
- "Most recent" is defined as **max `id`**: illuminations are only ever appended,
  so a higher id is necessarily a later run. (Not loader order — SeaORM's
  `EntityOrSelect::select()` orders related rows by primary key *ascending*, so
  `.first()` would have returned the **oldest** and shown stale data after a
  rerun.)
- The templates' `| first` then means "the latest", which is now true by
  construction rather than by luck.

> **Ordering is enforced in one place, deliberately.** Collapsing in `InfoMaker`
> means every consumer (templates, `ignition/util`, REST clients) gets the latest
> without repeating the rule.

---

## 8. Open questions / follow-ups

- **`task_run_status` retention (REVISIT):** add a cleanup/eviction policy to avoid
  unbounded table growth. Note the run dimension means a rerun task keeps *all*
  its rows, so growth is per-run rather than per-task. This is the reason the
  incomplete queries deliberately exclude `CompleteSuccess` (§5). *Tolerated — see
  `pragmatism.md`.*
- **Stuck tasks (REVISIT):** there is no heartbeat or timeout, so a task that is
  enqueued but never picked up (queue dropped, worker crash) stays `Queued`
  forever and looks active. Consider treating `Queued`/`InProgress` rows older
  than N minutes as dead, or a periodic sweep that stamps `CompleteFailure`.
  *Tolerated — see `pragmatism.md`.*
- **Incomplete queries have no `ORDER BY` (REVISIT):** they collapse to the
  latest run per logical task but return rows in non-deterministic order. Add an
  explicit order if the UI iterates the results. *Tolerated — see
  `pragmatism.md`.*
- **`create_run` is check-then-act (REVISIT):** `submit_inner` reads the latest
  run, then inserts. Two concurrent submitters can both pick the same run number.
  The `(envelope_id, run)` unique index arbitrates, so correctness does not
  depend on the read. *Tolerated — see `pragmatism.md`.*
- **`SparkTask.spark_id` is a placeholder (REVISIT):** `api/user/client.rs` mints
  a random `i32` (`uuid::Uuid::new_v4().as_u128() as i32`) because the real spark
  row id only exists after `insert_spark` runs at exec time. This makes spark's
  `envelope_id` non-deterministic. When spark gets a real identity (e.g. derived
  from its sorted `capture_ids`, or the planned "spark seed/spec" entity), it can
  join the deterministic scheme.
- **Spark is not queryable by capture (accepted for now):** a `SparkTask`
  operates on N captures but registers `entity_type = "spark"`, so a
  capture-scoped query will not surface it. **Accepted** — the plan is to
  introduce a "spark seed/spec" entity concept later. No N-entity join table is
  needed yet.
- **Envelope `user_id` is not validated against the capture owner (REVISIT):**
  `logic::spark::exec` derives `user_id` from the captures and never compares it
  to `envelope.user_id`, and `api/service/get_capture.rs` is explicitly **not**
  user-scoped. The webhook routes rely on Cloud Run OIDC, so this is
  defense-in-depth — but the envelope's `user_id` should be checked so status
  rows and data writes cannot diverge. *Tolerated — see `pragmatism.md`.*
- **Admin backfill mis-attributes task status to the admin (REVISIT — deferred to
  the backfill session):** two related problems, both currently masked by the app
  being single-user:
  1. **Attribution.** `api/admin/backfill.rs` passes the requesting admin's
     `context.user_id()` to `submit_search_index`, so every backfill task's
     `task_run_status` row is owned by the **admin**, not the capture's owner. The
     task itself still works (`logic::search_index::exec` fetches via
     `service_api.get_captures`, which is not user-scoped), but a user-scoped
     status view would **not** show the owner their own captures' backfill
     status.
  2. **Candidate selection is global.**
     `api/service/need_search_index.rs::get_captures_need_search_index` has **no
     user filter** — it joins capture + illumination, filters `archived_at IS
     NULL`, and orders by `created_at DESC`. So `--all` enqueues tasks for *every*
     user's captures, all attributed to the admin.

  **Why it's deferred:** fixing attribution properly means
  `get_captures_need_search_index` must return `(capture_id, user_id)` pairs (not
  `Vec<i32>`) and `backfill::enqueue` must group by owner — real design work that
  belongs in the dedicated backfill session. A half-fix would be worse than a
  documented gap. **Note:** `get_captures_need_search_index` also carries its own
  `TODO` — it returns recent captures without actually checking whether they need
  indexing, so candidate counts are inaccurate.
- **Backfill / bulk tasks (deferred):** the `background` flag was removed (never
  populated). Backfill handling — marking tasks as bulk, surfacing progress, an
  admin view, and fixing the `user_id` attribution + global candidate query above
  — gets a dedicated plan-and-branch session.
- **`/_wh/cloudtask/illuminate` route also serves future reruns:** the capture-create path
  submits `IlluminationTask` through the illumination queue, so the route has no
  live caller. Kept deliberately — it's the entry point for the future backfill
  and rerun flows. *Tolerated — see `pragmatism.md`.*

---

## 9. File map

| File                                      | Role                                                                                                                                                                                                  | Status |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| `src/task/task_def.rs`                    | `Task` trait (`task_type()`/`entity_type()`/`entity_id()`) + `TaskEnvelope<T>` (`user_id`, `envelope_id`, `run`, payload) + `TaskEnvelope::new(user_id, task, run)` / `make_envelope_id`              | ✅      |
| `src/task/taskqueue.rs`                   | `TaskQueue<T>` trait, enqueue-only (takes `TaskEnvelope<T>`)                                                                                                                                          | ✅      |
| `src/task/taskqueue_local.rs`             | `LocalTaskQueue` — in-process mpsc + semaphore backend (no retry)                                                                                                                                     | ✅      |
| `src/task/taskqueue_cloudtask.rs`         | `CloudTaskQueue` — Google Cloud Tasks backend                                                                                                                                                         | ✅      |
| `src/task/taskqueue_pubsub.rs`            | **removed** — Pub/Sub support stripped out; Cloud Tasks is the focus                                                                                                                                  | ✅      |
| `src/task/taskmaster.rs`                  | `TaskMaster` — public lifecycle/API boundary; owns queues and coordinates status persistence + notifications; `submit_*` / `begin_attempt` / `finish_attempt` / status query; shared via `Arc` | ✅      |
| `src/task/taskruntracker.rs`              | `TaskRunTracker` — private-to-task-module persistence component; creates/updates keyed by `(envelope_id, run)` and reads status rows | ✅      |
| `src/task/taskrunstatus.rs`               | `TaskRunStatus` enum + `is_in_flight()`/`is_incomplete()`; DB stores integer discriminant                                                                                                             | ✅      |
| `src/task/status_listener.rs`             | `StatusListener` — the `LISTEN`/`NOTIFY` thread (**stub**; see `sse.md`)                                                                                                                              | ⬜      |
| `src/task/beacon.rs`                      | **removed** — replaced by `TaskMaster`                                                                                                                                                                | ✅      |
| `src/model/task_run_status.rs`            | `task_run_status` SeaORM model (`(envelope_id, run)` unique, `entity_type`/`entity_id`, `status_code`, `attempts`) — auto-synced at startup                                                           | ✅      |
| `src/api/apierror.rs`                     | `ApiError::is_retryable()` — 5xx retryable, 4xx permanent                                                                                                                                             | ✅      |
| `src/config/schema.rs`                    | `Config.task_max_attempts` (env `TASK_MAX_ATTEMPTS`, default 3)                                                                                                                                       | ✅      |
| `src/webhook/http_status_for_task_run.rs` | `http_status_for_task_run` — maps `AttemptOutcome` to the HTTP status Cloud Tasks sees                                                                                                                | ✅      |
| `src/webhook/webhook_state.rs`            | `WebhookState` carries `task_master: Arc<TaskMaster>`                                                                                                                                                 | ✅      |
| `src/webhook/r_*.rs`                      | accept `TaskEnvelope<T>`; `begin_attempt`/`finish_attempt` around `logic::exec`; return `http_status_for_task_run`                                                                                    | ✅      |
| `src/logic/illuminate.rs`                 | `IlluminationTask` + `exec` (illuminate **and** index; no idempotency guard — §7)                                                                                                                     | ✅      |
| `src/logic/search_index.rs`               | `SearchIndexTask` + `exec` (no idempotency guard — §7)                                                                                                                                                | ✅      |
| `src/api/schema/infomaker.rs`             | collapses `illuminations` to the most recent (§7.1)                                                                                                                                                   | ✅      |
| `src/test_support/test_db.rs`             | schema-per-test DB harness (see `testing.md`)                                                                                                                                                         | ✅      |
