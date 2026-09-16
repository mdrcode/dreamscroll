# Testing philosophy

**Status:** living document. Last updated 2026-09-16.

> **See also:** `task-status.md` (the framework these tests cover),
> `pragmatism.md` (tolerated trade-offs), `sse.md` (future work).

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

Examples in the codebase: `StatusCode::incomplete_codes`,
`AttemptOutcome::from_failure`, `next_attempt_number`, `make_url`.

## DB tests

Use the harness in `src/test_support/test_db.rs`:

```rust
#[tokio::test]
async fn submit_illumination_records_a_queued_row() {
    let Some(db) = crate::test_support::test_db::test_db().await else {
        return; // no database available; skip
    };

    let service = TaskMaster::builder()
        .db(db.handle())
        .illumination_queue(queue)
        .build()
        .expect("build should succeed with a db");

    // ... assertions ...
}
```

### How isolation works

Each `test_db()` call creates a **fresh, uniquely-named schema**
(`test_<uuid>`) with the full app schema synced into it, and drops it on
teardown. So:

- Tests are fully isolated from each other.
- Tests can run **in parallel** safely.
- Nothing touches the app's real tables.

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
message on stderr, or check the test's runtime (a real DB test takes ~0.2s; a
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
