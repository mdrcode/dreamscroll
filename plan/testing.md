# Testing philosophy

**Status:** living document. Last updated 2026-11-12.

> **See also:** `plan/archive/task-status.md` (the framework these tests cover),
> `plan/pragmatism.md` (tolerated trade-offs), `plan/archive/sse.md` (implemented SSE delivery),
> `plan/smoke.md` (agent-assisted local smoke testing).

## The two tiers

Every test belongs to exactly one tier. The rule is simple: **if it needs a
database, it's a DB test.**

| Tier     | Attribute                                             | Requires | Runs                      |
| -------- | ----------------------------------------------------- | -------- | ------------------------- |
| **Unit** | `#[test]` / `#[tokio::test]`                          | nothing  | always, in parallel, fast |
| **DB**   | `#[tokio::test]` + `test_support::test_db::test_db()` | Postgres | when a DB is reachable    |

**Do not mock the database to make a DB test look like a unit test.** If the
thing under test talks to Postgres, test it against Postgres. Mocking a DB
tests your assumptions about SQL, not your SQL.

## Unit tests

Pure functions, in-memory logic, serialization, parsing, state machines. These
should be the majority of tests, and they should stay fast.

Examples in the codebase: `TaskRunStatus::is_in_flight`,
`AttemptOutcome::from_failure`, `next_attempt_number`, `make_url`.

## DB tests

Use the harness in `src/test_support/test_db.rs`. The knowledge tests open with one statement —
`db_or_skip!(db);` — which binds `db` or returns when no database is reachable. It is a *statement*
on purpose: the checked-out schema is returned to the pool when `TestDb` drops, so the handle must
outlive the test, not the expression that produced it.

```rust
#[tokio::test]
async fn submit_illumination_records_a_queued_row() {
    let Some(db) = crate::test_support::test_db::test_db().await else {
        return; // no database available; skip
    };

    let service = TaskMaster::builder()
        .db(db.handle())
        .illuminate_queue(queue)
        .build()
        .expect("build should succeed with a db");

    // ... assertions ...
}
```

### How isolation works

`test_db()` hands each test a **schema of its own** (`test_<pid>_<uuid>`) with the full app schema
synced into it. So:

- Tests are fully isolated from each other.
- Tests can run **in parallel** safely.
- Nothing touches the app's real tables.

Schemas are **pooled**, not created per call. Building one costs ~115ms (`CREATE SCHEMA`, a pool,
`sync_schema` over 21 tables), and creating and dropping them takes catalog locks, which is why the
suite used to spend all of its wall time there and ran *slower* on 8 threads than on the default.
Instead a schema is built once, reset between tests with a single `TRUNCATE ... RESTART IDENTITY`
(~21ms), and returned to a process-wide pool in `Drop` — synchronously, because the old teardown
handed the `DROP SCHEMA` to the test's own runtime, which died first, so every schema leaked and
each run paid to sweep the previous run's 126.

The reset also rolls back a transaction a panicking test may have left open, re-pins the search path,
and puts back the `dreamscroll_schema_version` marker that the truncate removes.

Two consequences worth knowing:

- A test may not create its own tables: the truncate list is captured once per schema, so extra
  tables (and their rows) would survive into the next test that draws that schema.
- A schema carries no connection. A `DatabaseConnection` belongs to the runtime that built it, and
  every `#[tokio::test]` brings its own — sharing one across runtimes deadlocks, because the pool's
  background tasks died with the first runtime. Each test connects for itself (~25ms).

Measured on this machine: the lib suite went from 16.4s to 6.2s wall, and 8 threads from 20.4s to
5.6s — contention gone rather than amortized.

### Why this design (and not the alternatives)

| Approach                   | Why not                                                                                                                                                                                                   |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **sea-orm `MockDatabase`** | It asserts on the SQL you *expect* to emit, rather than exercising real queries. Brittle, and it wouldn't catch a wrong query.                                                                            |
| **`sqlx::test`**           | It wants a `migrations/` folder. This project creates its schema via sea-orm's `schema-sync` at startup, so there are no migrations to apply. The harness reuses `schema-sync` instead of duplicating it. |
| **Transaction rollback**   | `DbHandle` holds a concrete `DatabaseConnection`, and a `DatabaseTransaction` isn't one. Making it generic over the executor would ripple through every repository.                                       |

### Skipping gracefully

`test_db()` returns `None` when no database is reachable, and the test returns
early. This keeps `cargo test` **green on machines without Postgres** (and in CI
without a DB service), while still running the DB tests wherever a DB exists.

The trade-off: a skipped DB test looks like a passing test. If you need to be
sure DB tests actually ran, watch for the `test_support::test_db: skipping DB test, ...`
message on stderr, or check the test's runtime (a real DB test takes ~50ms; a
skipped one is instant).

## Test support modules

`src/test_support/` holds shared test infrastructure (`cfg(test)` only):

| Module        | Purpose                                                           |
| ------------- | ----------------------------------------------------------------- |
| `test_config` | `load_config()` — the app's config, loaded once per test process. |
| `test_db`     | `test_db()` — the isolated-schema database harness.               |

`test_config` is deliberately separate from `test_db`: config is a
cross-cutting test concern, so any future test module can use it without
depending on the database harness.

## Running DB tests

The harness loads the app's config (`config_local.env` + `.env`) via
`test_support::test_config::load_config` and builds the connection URL with
`database::make_url_from_config` — the same code path the app uses. So if the
app can reach Postgres, so can the tests. No extra env vars needed.

```bash
# Local dev: Postgres is already in docker-compose on :5432
docker compose up -d db

cargo test
```

## ⚠️ Required DB permissions

**The DB user must have `CREATE` permission on the database** (to create and
drop the per-test schemas). This is an external requirement that is easy to
forget when provisioning a new environment.

- **Local (`docker-compose.yaml`):** the `dreamscroll_pg_user` is the database
  owner, so this works out of the box.
- **Prod / Cloud SQL:** the app user does **not** need this — DB tests are
  `cfg(test)` only and never run in production. But if you ever run the test
  suite against a Cloud SQL instance, that user will need `CREATE` on the
  database, or the harness will skip (with a clear message).

If `test_db()` reports "cannot create schema", this permission is the first
thing to check.

## Conventions

- **Name DB tests for the behavior**, not the mechanism:
  `submit_illumination_records_a_queued_row`, not `test_record_1`.
- **One behavior per test.** The harness makes setup cheap, so there's no reason
  to bundle assertions.
- **Prefer pure functions.** If logic can be extracted into a pure function
  (like `AttemptOutcome::from_failure`), do that and unit-test it — it's faster
  and clearer than a DB test. Reach for the DB only when the DB *is* the thing
  under test.
- **Don't assert on row counts across the whole table.** Assert on the specific
  rows you created. (Isolation makes this safe, but it keeps tests readable.)
- **Open a DB test with `db_or_skip!(db);`.** One statement that binds `db` or
  returns when no database is reachable — see *DB tests* for why it is a
  statement, not an expression.
- **Name the test for the claim it can fail on.** If the name says "the best
  candidate wins", the assertion must demand the specific one, not either one.

## What a test has to be able to fail on

An audit of the whole suite (2026-11-05) found the recurring ways a test can hold
a line count while catching nothing. Each one is worth checking for by name:

- **A tautology.** `identity()` forwarding to `key()`, a function returning the
  constant it is defined as, `derive(Clone)` then asserting a clone equals its
  source, asserting a private default function equals the default it is.
- **A fixture that cannot distinguish.** A parser test with `{"clusters": []}`
  passes when every field name is renamed. An "excludes before limiting" test
  whose data never exceeds the limit passes if exclusion happens after. Feed the
  case that separates correct from incorrect.
- **An input equal to the default.** `env` var set to exactly the default value
  cannot detect an ignored variable. Override with something that differs.
- **A weak assertion.** `assert!(!all(reserved))` accepts items still reserved;
  `is_ok()` as the only claim; `contains` on a substring implied by an equality.
- **An unreachable pin.** A test for a path no caller can reach today documents a
  non-goal; put the rationale in a comment on the code instead.
- **Duplicate coverage.** The same fact at two layers (a pure helper and the DB
  path that calls it) is one fact. Keep the layer a consumer actually exercises.

The failure mode these share: **the test would still pass if the behaviour it
names were deleted.** Ask that question of every test you keep.

## Open coverage gaps — SSE/task (rewritten 2026-11-12)

Six of the nine gaps recorded on 2026-10-08 now have tests that fail if the behaviour is deleted;
they are listed at the end rather than kept as gaps. What remains is what breaks *silently* — a
render nobody looks at, a stream that stays open while nothing drives it, a refetch contract
nothing pins — which is why it is still open:

1. **Authenticated partial routes.** Nothing renders a capture card or the detail partial against a
   real session, so a missing or another user's capture, and ownership leakage, are checked by hand
   (`plan/smoke.md`). The web tests today are parsing and helper tests (`r_events`) plus one
   redirect check (`r_capture`). Gate: the backlog's deferred table names this as "next change to
   auth or route-level authorization".
2. **Listener fan-out death.** `sse/listener.rs` logs and exits when `recv` fails, and says so: *"a
   dead fan-out looks like an idle stream until restart"*. Heartbeats keep coming from the stream
   loop, so a client cannot tell it from an idle page either, and the only recovery is a reload.
   Needs a test that kills the listener and asserts the stream ends (or the client is told to
   reconnect), then the liveness fix. This is also where the real `NOTIFY → PgListener → stream`
   seam would be covered: today the publisher is tested through a recording notifier, and the
   decoder by feeding it rows.
3. **Catch-up fan-out.** `StatusCatchup` collapses to the latest run per `(entity, task type)`, but
   nothing asserts that two task types on one entity produce a single initial refresh hint while
   live updates stay per-event — nor the subscribe-before-snapshot handoff. Cheap as a contract
   test, and the refetch cost the design trades on is what it protects.

Closed since 2026-10-08, kept so they are not rediscovered as gaps: the notification lifecycle
(`successful_status_writes_publish_events_but_conflicts_and_missing_updates_do_not`, plus the
enqueue-failure and attempt-lifecycle tests in `taskruntracker`/`taskmaster`), the catch-up endpoint
(`catchup_replays_latest_status_with_an_estimate_only_while_running`,
`stream_emits_catchup_then_user_wide_live_events`, the `r_events` cap/dedupe/skip tests), task edge
cases (`first_attempt_of_unknown_task_is_one`, `submit_after_completion_starts_a_new_run`,
`submit_spark_requires_at_least_one_capture`), duplicate deliveries
(`duplicate_submit_while_in_flight_is_refused`,
`concurrent_duplicate_submissions_enqueue_exactly_one_run`,
`completed_task_is_not_resurrected_in_db`, `set_status_touches_only_its_own_run`), and the
illumination retry path (`insert_persists_v1_raw_only_and_v2_projection`, which asserts that a
retry replaces the previous result).

The browser-lifecycle gap had no coverage to close: it moved to
`plan/archive/testing-js-and-browser.md`, gated on a client bug that unit tests miss.
