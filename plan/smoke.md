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

The dedicated account is `smoke-agent`. Create it once with the normal admin
command, then grant it admin so UI *and* admin REST checks can run as the same
account (local database only):

```bash
cargo run --bin dreamscroll_db -- create_user
cargo run --bin dreamscroll_db -- set_admin smoke-agent
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

`smoke-agent` is an admin in the local database, so admin REST commands run as
`smoke-agent` rather than as the owner. The flag is granted explicitly, once per
database (local only; prod accounts keep their own flags):

```bash
cargo run --bin dreamscroll_db -- set_admin smoke-agent          # --revoke undoes it
```

`set_admin` is a direct-database tool like `sync_schema`: the DB credentials are
the gate, not an admin login, so it also works to recover admin access.

`dreamscroll_api` does not read `smoke.env`, but it does read
`DREAMSCROLL_API_TOKEN`. With that set it never touches the OS keychain — no
read, no write — so runs do not trigger a keychain permission prompt, and
`--user` is optional (the name is taken from the token for display):

```bash
set -a; . smoke.env; set +a
TOKEN=$(curl -s -X POST "$SMOKE_BASE_URL/api/token" -H 'Content-Type: application/json' \
  -d "{\"username\":\"$SMOKE_USERNAME\",\"password\":\"$SMOKE_PASSWORD\"}" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["access_token"])')

DREAMSCROLL_API_TOKEN="$TOKEN" ./target/debug/dreamscroll_api --host localhost:8080 \
  export-entities --out /tmp/entities.jsonl
```

`--user` is optional here and does not select the account: passing it anyway
prints a warning naming who the token actually belongs to. A rejected token is
an error, not a prompt: unset the variable to authenticate interactively.

Without the variable the CLI falls back to a JWT cached in the OS keychain,
keyed by host + username, and logs in at the `Enter password:` prompt (rpassword
reads the tty, so piping the password does not work). macOS asks for keychain
permission on first use of each binary, which is why the environment variable is
the default in this runbook.

Never mint, inject, or seed a token into the keychain to skip the prompt. A
hand-built JWT bypasses credential verification, which is the one property the
auth layer exists to guarantee; the cached token must be one the server issued
for credentials somebody actually presented.

## Boundaries

- Use only the local host (`127.0.0.1` / `localhost`).
- Use the `smoke-agent` account, not the owner's account, for UI and HTTP checks,
  admin REST commands included. Prod admin commands still need an account whose
  token only its owner can cache.
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
