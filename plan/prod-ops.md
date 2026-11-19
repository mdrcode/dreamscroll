# Production operations — friction log and proposals

**Status:** Research — active (2026-11-18). Written after the knowledge v2 cutover, the longest
production session so far. The release itself was fine; the *reading* of production was not.

**Purpose:** turn what this session had to improvise into named, read-only commands. The theme is
that privileged DB access already exists in-region (the `dreamscroll-db-sync` job, with the
`/cloudsql` socket and the password secret). Every inspection that instead went through a laptop
proxy, a dumped secret, or ad-hoc SQL is a gap in that tooling, not a fact about Postgres.

**How this relates to [`prod-deploy.md`](prod-deploy.md):** that doc is *normative* — the release path
and runbook as they exist today, and the place a rule lives. **This doc is not a runbook.** It is
*input*: what the 2026-11-18 session had to improvise, and the tooling it should have had.

> If you are about to touch production, read [`prod-deploy.md`](prod-deploy.md). Nothing below is
> implemented until it appears there; do not follow a proposal as if it were procedure.

When a proposal ships, its recipe moves into `prod-deploy.md` (or `gcloud/`) and the proposal is
deleted here; when the list is empty this doc goes to `plan/archive/`. Never state the same rule in
both places — the runbook wins.

## What the session actually required

| Need | What I did | Cost / risk |
| --- | --- | --- |
| Prod schema marker and table shape | Started a Cloud SQL Auth Proxy container, fetched the DB password from Secret Manager, ran `psql` | ~10 min setup; a static proxy token expired mid-run and had to be replaced with ADC; credential handling on the laptop |
| Would the 8→43 sync succeed? | `pg_dump` (17 MB) → restore into a local PG18 copy (112 s) → run `sync_schema` → read the error (`alt_types contains null values`) → write schema 43 → re-run | ~30 min and two iterations; the dump is the only safe way to answer it today |
| Does prod have the queues this release will submit to? | `gcloud tasks queues list` after reading the plan's warning | the manifest is the *next* state and legitimately led prod; the gap is that the release path did not assert the external prerequisite, so the only pointer was a plan checklist |
| Manifest vs live drift | `prod-apply.sh --check` | this worked; it is the model to copy |
| Which env vars does the binary require? | manual three-way diff of `config_def.rs`, the manifest, and `config_prod.env` | `config_prod.env` is gitignored, so the manifest is the only tracked source; `prod-apply.sh` keeps a third list |
| Rebuild the historical graph | local `dreamscroll_knowledge link` through the proxy | 56 ms per statement; 3,690 observations took ~40 min. The same remainder ran in-region in 79 s |
| Why did an upload fail? | `gcloud logging read` until the `ApiError:` line appeared | the API body is opaque by policy, so the browser showed only "Upload failed" |

## Proposals, ranked by leverage

### P1 — schema audit (status and dry run shipped 2026-11-18)

`dreamscroll_db status` and `sync_schema --dry-run` now exist and run in-region through the existing
job (`--args status`, `--args sync_schema,--dry-run`); both are documented in `prod-deploy.md`. What
remains is **`audit`**: the model-registry vs `information_schema` column diff, so a surviving table
with retired `NOT NULL` columns is reported by the same command instead of a hand-built CTE.

### P2 — one read-only status command for a release

`gcloud/prod-status.sh` (or `prod-apply.sh --status`), one invocation, no writes:

- drift (the existing `--check`);
- service: revision, image digest, traffic, ingress;
- job: image and the last executions;
- queues: all five, state, concurrency against the expected table;
- latest backup and its age;
- count of `severity>=ERROR` in the last hour;
- optional `--db`: execute P1's `status`.

Today this was ~15 ad-hoc `gcloud` calls, each with its own format string and failure mode. Exit 1 on
anything unexpected, so it can be the first command of every session.

### P3 — release preflight for what the manifests only *name*

The manifests describe the **next deployed state**, and they are right to lead prod: a queue name, a
secret reference, or an OIDC audience is declared there for a release that has not happened yet.
Nothing about that is drift. What is missing is that those things live outside Cloud Run — Cloud
Tasks, Secret Manager, IAM — so `--check` compares them against nothing, and the knowledge that they
must exist lived in a plan checklist.

Make it a release assertion instead. On `--apply` (and in `--check` when it can read them):

- every `TASK_QUEUE_NAME_*` in the manifest names an existing queue, with the expected concurrency
  (a small table beside `prod-config.sh`);
- every `secretKeyRef` names a secret with an enabled latest version;
- the job's Cloud SQL instance exists and is reachable through the same annotation the manifest uses.

Fail before applying, so the first failing signal is a preflight line, not the first capture after a
successful-looking deploy. Create the queues/secrets in the same release step as the image, as the
2026-11-18 release did.

### P4 — required env from one source

`Config` knows the required variables; `prod-apply.sh` hand-maintains `REQUIRED_WEB_ENV`; the manifest
is the tracked deployment truth. Three lists, one of them untracked (`config_prod.env`). Expose
`Config::required_env_names()` and assert in a test that the manifest carries every name, so a new
required variable cannot ship as a boot-time surprise.

### P5 — error codes for the UI

`ApiError` deliberately hides its message from clients (it can carry provider bodies), so the UI can
only say "Upload failed. Please try again." Today that turned a precise 413 into a mystery for two
upload attempts. Add a stable, non-sensitive **code** (`upload_too_large`, `rate_limited`,
`unauthorized`, …) to error responses and map codes to copy in the UI. A code is a classification,
not an internal message; it cannot leak a provider's response body.

### P6 — operator job wrapper

The in-region recipe is now in `plan/prod-deploy.md`, but it is still create/execute/logs/delete by
hand, with a digest to pin and a one-writer rule to remember. `gcloud/operator-job.sh <command>
[args…]` should pin the current commit's digest, refuse if a job is already running, execute with
`--wait`, print the report line, and delete the job.

### P8 — provider health visibility (rate-limit event shipped 2026-11-18)

The gateway now emits `event="provider.rate_limited"` with `provider`, `kind` and `retry_after_secs`,
and `src/telemetry/prod_event_registry.rs` is the canonical vocabulary (an operational contract, not
plumbing). The log-based metric `provider_rate_limited`
exists (2026-11-18) with a `provider` label extracted from `jsonPayload.provider`, so Wikidata is
identifiable without a second metric. Remaining: put the fetch counters and the refusal reason on the
online resolve log line, add a per-provider `found`/`empty`/`failed`/`degraded` block to `status`,
and attach an alerting policy — the policy needs a notification channel.

Before the event existed, a 429 left no trace beyond `refused=true`, and a failed fetch left only a
cache row. Measured 2026-11-18, a three-hour window that included the drain: 2,898 `found` + 1,143
`empty`, **no `failed` or `degraded`**, zero `refused=true` across five online resolves, cache 4,041
rows and 0 expired — healthy, but provable only by pulling two ad-hoc queries. A later 3,268-fetch
slice did record exactly one failed search (`United States Census Bureau`), with no status or error
anywhere; that is the shape the event should remove.

## What not to build

- **An arbitrary-SQL HTTP route.** It concentrates a credential and a query surface for convenience.
  The job already runs with the credential; give it named, read-only commands instead. If a browser
  view is ever wanted, expose the *same report* as an admin-only GET — not a query console.
- **A second release path.** Manifests + `prod-apply.sh` are why today's drift was one diff and the
  rollback was a re-apply. Cloud Build or the console would reintroduce the drift the cutover had to
  clean up.
- **Sync at service startup.** The explicit job is why every failed jump rolled back with the old
  revision still serving.

## What already worked

The release path itself did not need reinvention: checked-in manifests with `prod-apply.sh --check`,
a single-transaction `sync_schema`, and the job pattern for privileged DB work all earned their keep
and are documented in `prod-deploy.md`. This doc only adds what was missing around them.

## Lessons for the next session

1. **Read the marker first.** `plan/prod-deploy.md` said 33; production was 8. A status command (P1)
   makes this the first command instead of a discovery.
2. **Rehearse a schema jump by rolling back, not by restoring a dump** (P1's dry run), unless you also
   need to exercise the data migration.
3. **Latency × statement count is the cost model for operator CLIs.** 56 ms through the proxy turned
   an N+1 linker into an hour. Run graph commands in-region.
4. **One writer at a time.** `link` from two places would create competing entities; the job recipe
   says so, and P6 should enforce it.
5. **A release is not done when the revision is ready** — it is done when the derived data is rebuilt
   (`link`) and the online chain has run once on a real capture.
