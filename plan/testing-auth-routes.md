# Testing Authenticated Routes — Follow-up Plan

**Status:** Planned follow-up to the `sse` branch; not implemented yet.
**Goal:** Exercise protected WebUI routes through the real Axum authentication/session middleware using a real, isolated Postgres test database.

## Approach

Use the existing `crate::test_support::test_db::test_db()` fixture to create a per-test schema. Seed a real test user and any required domain rows (for example captures and task status). Construct the router with the real `WebAuthBackend`, `AuthManagerLayerBuilder`, and `login_required` middleware, and use `tower_sessions::MemoryStore` for the session store. `MemoryStore` is already available through the existing `tower-sessions` dependency; no new dependency or auth bypass is needed.

Log in through the normal route/auth flow to obtain the session cookie, then send requests through the in-process Axum router using `tower::ServiceExt::oneshot`. Attach the cookie to requests that should be authenticated. Also send requests without a cookie to verify protected-route redirects/denials.

## Initial coverage: `/events`

Add a DB-backed authenticated route test that:

1. Seeds task status rows for the test user and another user, across requested and unrequested capture IDs, multiple task types, multiple runs, and different statuses.
2. Opens `/events?capture_ids=...` with the authenticated cookie.
3. Reads the initial SSE catch-up response and asserts it contains only the authenticated user's requested captures, latest status per logical task, and at most one catch-up event per entity after snapshot coalescing.
4. Confirms live-stream filtering and shutdown with the existing focused stream tests; avoid turning this first route test into a long-lived listener integration test.
5. Verifies an unauthenticated request follows the configured protected-route behavior.

## Follow-on routes

Apply the same harness to `/cards/capture/{id}` and `/detail/{id}/partial`: assert rendered partial content for an owned capture, not-found behavior for missing/archived/other-user captures, and no cross-user data leakage.

## Test boundaries

- Keep database behavior on the real isolated Postgres schema; do not mock the ORM.
- Keep auth/session middleware real; do not call protected handlers directly with a fabricated `AuthSession`.
- Keep pure stream ordering, user filtering, and shutdown behavior in the existing unit tests.
- Use the in-memory session store only for tests, not as an application runtime configuration.
- DB tests may use the established graceful-skip convention when Postgres is unavailable; verify the test actually ran when validating locally.

## Validation

Run the targeted route test and the full library test suite. Confirm the DB test did not skip, then run `cargo check --all-targets`.
