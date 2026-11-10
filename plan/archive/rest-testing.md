# Testing the REST API

**Status:** Planned — design only, nothing implemented. `make_api_router` has no test coverage.
**Last updated:** 2026-11-05.

> **See also:** [testing.md](../testing.md) (the standard every test here must meet),
> [testing-coverage.md](testing-coverage.md) (umbrella strategy; this doc is its REST-surface
> section), [testing-auth-routes.md](testing-auth-routes.md) (the WebUI/session surface — same
> `oneshot` technique, different auth mechanism), [testing-js-and-browser.md](testing-js-and-browser.md)
> (browser-level work). `src/rest/_local.http` and `_prod.http` are the documented contract.

## Where the surface stands

| | |
|---|---|
| routes | 28, across 17 files under `src/rest/` (`make_api_router`) |
| handlers | 27 `pub async fn` |
| handler tests | 2, both request deserialization in `r_queue.rs` |
| router tests | **0** — the route table, its nesting, and which routes are open are all unasserted |
| auth | per-handler `DreamscrollAuthUser` extractor (JWT bearer, session via axum-login); no middleware layer |
| service layer | well covered — ~1,080 test LOC in `api/user`, called directly, not through HTTP |
| client | `rest::client`: 1 test (bearer header); response and error parsing untested |

The business logic behind the API is tested. The API itself — routing, auth gating, extraction,
status codes, wire shapes — is not.

The motivating example: `r_queue::accepted_task_run_response` answers `202 Accepted` with a bare
JSON run id, and three documents say so (`_local.http:63`, `_prod.http:61`,
`plan/archive/rest-task-submission.md:73`). Its only test compared the function's output to the
constants it is written with, so the 2026-11-05 test audit deleted it; nothing now asserts the status
or the body of either queue endpoint.

## Layers, and where a test belongs

| layer | owns | test here? |
|---|---|---|
service (`api::user::*`, `api::service::*`) | behaviour and persistence | already covered; **do not duplicate** |
handler (`rest::r_*`) | translation: extract → service call → response | only where extraction is non-trivial and the router cannot reach it cheaply |
router (`make_api_router`) | path → handler, auth gate, status, body shape | **this doc's subject** |
client (`rest::client`) | parses responses, maps errors | open (see *Open questions*) |

The rule that keeps this cheap: **assert the documented wire contract at the router; leave behaviour
to the service tests.** A handler thin enough to need a test of its own is a handler doing too much.

## The harness: `test_support::rest`

~70 lines, mirroring `dreamscroll_web::initialize` exactly, and reusing what already exists for
storage and users.

```rust
let rest = test_support::rest(&db).await;          // Router + JwtConfig, built as production does
let token = rest.token_for(owner_id);              // a real JWT, minted through auth::jwt
let res = rest
    .request(Method::POST, "/api/queues/illuminate")
    .bearer(&token)
    .json(&json!({"capture_id": 42}))
    .send()
    .await;
assert_eq!(res.status(), StatusCode::ACCEPTED);
```

Builds: `RestState { user_api, admin_api, jwt_config, task_backend, max_upload_bytes }` with the test
storage provider and the fixtures for a user and capture; `tower::ServiceExt::oneshot` over the real
router, so extractors, JWT validation and status mapping are all production code.

Explicitly **no bypass**: no fabricated `AuthSession`, no `TestStorageProvider` special-casing in
handlers, no mocked database. Unlike [testing-auth-routes.md](testing-auth-routes.md) this needs no
`MemoryStore` — the REST API authenticates by bearer token only.

## Four kinds of test

| kind | pins | size | why |
|---|---|---|---|
**Auth matrix** | every protected route without a token → 401; admin route with a non-admin token → 403; `/token` open | ~30 | catches the structural bug that matters: a route moved outside the protected group. Highest value per line in the suite. |
**Route inventory** | the `(method, path)` set served equals an explicit list | ~25 | a dropped or renamed route fails; the list is the API spec, written twice on purpose |
**Contract per route** | authenticated request → status + JSON shape | ~10–15 each | what consumers depend on; this is where 202 + run id lives |
**Error paths** | 400 bad input, 404 missing, 409 in flight, 413 too large, 415 unsupported | ~5–8 each | the promises the docs make and nothing checks |

Not tested here: business logic, `IntoResponse` formatting beyond the contract, serde round-trips of
DTOs (covered by `api/schema/*`).

## The `.http` files are the spec

`_local.http` / `_prod.http` carry executable documentation ("Returns 202 Accepted with the JSON
run_id string, 409 Conflict if in flight, or 404"). A test asserts each promise it makes. The
corollary: **a documented promise with no test is either a test to write or a doc line to delete** —
no third option.

## Order of work

Chosen by blast radius, not by file order:

1. `queues/illuminate`, `queues/search_index` — async submission (202/409/404), documented, consumed by the CLI
2. read paths: `captures`, `entity`, `entity/{id}`, `timeline`, `tasks/{run_id}`
3. mutations: `captures/import`, `captures/{id}/archive`, `captures/{id}` DELETE, `annotation`
4. `admin/*` — the authentication/authorization boundary for everything administrative
5. `token`, `account/password` — login and credential changes
6. `export/{kind}`, `dummy` — largest payload, trivial

## Phases

| phase | contents | size |
|---|---|---|
1 | `test_support::rest` + auth matrix + route inventory | ~125 lines |
2 | contract tests for the read routes (6–8) | ~100 lines |
3 | mutations + error paths (409/400/404/413/415) | ~120 lines |
4 | client parsing and error mapping (needs a decision, below) | ~60 lines |

Each phase is a commit; each verifies with `cargo test --lib rest`, the full suite, and clippy — and
confirms the DB tests actually ran rather than skipped.

## Decisions

1. **The `.http` files are the contract**, asserted literally (status codes, JSON field names). *Proposed: yes.*
2. **Route inventory is an explicit list in the test**, not derived from `maker.rs`. *Proposed: yes* — a spec written twice fails when one copy changes.
3. **Real minted JWTs**, never a fabricated user. Exercises `auth::jwt`, which is the production path.
4. **Status codes asserted as literals** (`202`), not as the symbolic constant the handler returns — otherwise the assertion cannot fail (the mistake that got the original test deleted).
5. **Handlers are not unit-tested for wiring.** If a handler needs its own test, it is doing more than translation.

## Non-goals

- No changes to service-layer tests; no duplication of `api/user` coverage.
- No mocking of Postgres, storage, or auth.
- No browser/JS coverage ([testing-js-and-browser.md](testing-js-and-browser.md)).
- No WebUI session routes ([testing-auth-routes.md](testing-auth-routes.md)); they use the same
  `oneshot` technique with a different auth mechanism and warrant their own fixtures.
- No line/branch coverage target ([testing-coverage.md](testing-coverage.md) §coverage tools).

## Open questions

1. **Multipart and the size limit.** `captures/import` takes an upload and `max_upload_bytes` yields
   413. Test through the router (needs a multipart body builder) or unit-test the limit check?
2. **Streaming bodies.** `admin/export/{kind}` streams. Assert status only, or drain and assert the
   first record?
3. **Client tests.** `rest::client` speaks reqwest over a URL. Test it against the in-process router
   on an ephemeral port, or leave it to the smoke procedure ([smoke.md](../smoke.md))?
4. **SSE and media routes** live outside `make_api_router`; do they ever share this harness?
