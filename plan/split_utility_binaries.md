# Utility binaries split

**Status:** Implemented 2026-09-21.

## Goal

Replace the catch-all `dreamscroll_util` binary with two utilities that make
transport and dependency boundaries explicit:

- `dreamscroll_api` always talks to a configured Dreamscroll host through REST.
- `dreamscroll_admin` connects directly to the database and local services, as
  an administrative/server-side tool.

The long-term direction is for `dreamscroll_api` to grow as supported REST
operations are added, while `dreamscroll_admin` shrinks toward only the small
set of operations that genuinely require direct access.

## Why

The old `dreamscroll_util` mixed two incompatible execution models. REST
commands carried database and service dependencies, while direct commands used
an empty `TaskMaster` as a workaround. That made it easy for a command to
silently depend on the wrong transport or to report task submission without a
real queue.

The split keeps the exploratory utilities useful without weakening the core
`TaskMaster` contract. In particular, REST commands no longer need a database
connection or any `TaskMaster` instance.

## Current binaries

### `dreamscroll_api`

REST-only. Requires a host and optionally a username. It owns REST
authentication and token-cache behavior.

Commands:

- `backfill`
- `change_password`
- `clear_token`
- `export_digest`
- `import_digest`
- `illumination_text`
- `spark`

### `dreamscroll_admin`

Direct database/service access. It does not accept or initialize REST host
configuration.

Commands:

- `create_user`
- `enums`
- `first_user`
- `hash_password`

The old `dreamscroll_util` binary was removed.

## State types

`src/util_cmds/cmd_state.rs` now contains two separate state objects:

- `ApiCmdState`
  - application configuration
  - REST host and optional username
  - lazy REST client initialization
  - token cache handling
  - no database, storage provider, user API, service API, or `TaskMaster`
- `AdminCmdState`
  - application configuration
  - direct database handle
  - lazy storage provider
  - direct user/service API clients
  - no REST host or REST client

Commands accept the state type matching their transport boundary.

## Related changes

- `Dockerfile` builds and packages `dreamscroll_web`, `dreamscroll_api`, and
  `dreamscroll_admin`; the runtime image includes all three.
- The manual image build/push helper and Cloud Build config now live under
  `gcloud/`. `gcloud/docker-build-push.sh` resolves the repository root from
  its own location, explicitly selects the root `Dockerfile`, and uses the root
  as the Docker build context, so it can still be invoked from the repository
  root (or another working directory).
- Database startup guidance now recommends
  `dreamscroll_admin first_user`.
- Direct admin user creation calls the database-backed admin operation without
  constructing a `TaskMaster`.

## Remaining follow-up

The direct admin command set no longer constructs a `UserApiClient` or an
empty `TaskMaster`. User-facing search commands use REST through
`dreamscroll_api`; synchronous illumination and indexing utilities were
removed rather than preserving a second execution path.

## Validation

- `cargo check --bins` passes.
- The current Docker image builds successfully for `linux/amd64` with all three
  packaged binaries.
- Current suite: `cargo test` passes with 189 tests; `cargo check --all-targets`
  passes.
