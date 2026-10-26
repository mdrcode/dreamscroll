# Smoke Testing a Running Local Instance

**Status:** Local manual smoke procedure for the persistent local environment.

**Scope:** How to exercise the normal local app with the dedicated `smoke-agent`
account. This is not a CI suite and does not replace `plan/testing.md`.

**Goal:** Verify rendered UI and client behavior against the persistent local
environment without using the owner's personal account.

---

## Setup

The local app uses the normal `config_local.env` + `.env` configuration and runs
at:

```text
http://127.0.0.1:8080
```

Start it normally:

```bash
docker compose up -d db fake-gcs
cargo run --bin dreamscroll_web
```

The dedicated non-admin account is `smoke-agent`. Create it once with the normal
admin command:

```bash
cargo run --bin dreamscroll_admin -- create_user
```

Credentials live in the gitignored root `smoke.env`:

```text
SMOKE_BASE_URL=http://127.0.0.1:8080
SMOKE_USERNAME=smoke-agent
SMOKE_PASSWORD=...
```

Load them only in the shell that needs them:

```bash
set -a
. smoke.env
set +a
```

---

## Interaction

- **Browser:** Open `/login`, sign in normally, and exercise feed, detail,
  entity, and responsive behavior. This is the preferred mode for UI checks.
- **HTTP:** Use a cookie jar and the real CSRF flow from `/login` for status and
  HTML checks. This does not execute JavaScript.
- **Database:** Read-only `psql` queries are fine for checking what should be
  displayed.

Do not bypass authentication or inject session cookies.

---

## Admin commands via `dreamscroll_api`

`dreamscroll_api` does not read `smoke.env`. It authenticates with a JWT cached in
the OS keychain, keyed by host + username, so the credential is specific to
`localhost:8080` and the account that minted it. Only an admin account can call
`/api/admin/*`; `smoke-agent` cannot.

An agent cannot type the owner's password, so the flow is:

1. Build the binaries the run needs:
   ```bash
   cargo build --bin dreamscroll_api --bin dreamscroll_web
   ```
2. Start the local server (see Setup) and leave it running — the token is cached
   against a reachable host.
3. Ask the user to cache the token themselves, by running one authenticated
   command and entering their password at the `Enter password:` prompt:
   ```bash
   ./target/debug/dreamscroll_api --host localhost:8080 --user <admin> \
     backfill status <backfill-id>
   ```
   Take their confirmation that it succeeded; do not inspect the keychain.
4. Run the admin command. The cached token is reused without a prompt:
   ```bash
   ./target/debug/dreamscroll_api --host localhost:8080 --user <admin> \
     backfill failures <backfill-id>
   ```

Never mint, inject, or seed a token into the keychain to skip the prompt; the
cached credential must be the user's own. Prefer read-only admin commands
(`status`, `errors`) for verification, and follow the usual ask-first rule for
anything that mutates state.

---

## Boundaries

- Use only the local host (`127.0.0.1` / `localhost`).
- Use the `smoke-agent` account, not the owner's account, for UI and HTTP checks.
  Admin REST commands are the exception: they need an admin account whose token
  only the owner can cache (see Admin commands above).
- Search, uploads, illumination, annotation edits and deletes, and capture
  archive/delete can mutate data or call paid providers; ask before using them.
- The account and its data are persistent. There is no automatic cleanup.

---

## Core checks

- Login and logout through the normal CSRF flow.
- Feed cards render with images, summaries, and entity pills.
- Entity pills navigate to `/entity/{id}`.
- Detail page opens; disclosures expand; collapsed content is inert.
- Entity occurrence page shows metadata, source capture, and sibling entities.
- Desktop and mobile layouts do not overflow.
- No browser console errors.
