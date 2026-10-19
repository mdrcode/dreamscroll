# Schema synchronization and migration plan

**Status:** application code implemented; Cloud Run Job deferred. **Updated:** 2026-10-16.

## Decision

Move application-model schema changes out of normal service startup and into an
explicit `dreamscroll_admin sync_schema` operation. Keep the first implementation
model-driven with SeaORM's schema registry and put its orchestration and manual
index ownership under `src/database/`. Do not add a migration dependency for
this first step.

This matches the current prototype's need for a manually controlled update
without turning startup into a database DDL operation. It is not a versioned
migration system; that tradeoff and the trigger for adopting one are recorded
below.

## Current behavior and failure

Before this work, `src/database/postgres.rs::connect_postgres_db` ran
`get_schema_registry("dreamscroll::model::*").sync()` and then
`sync_task_run_indexes()` on each connection setup. `database::connect()` also
ran the Tower Sessions store migration. Admin command setup used that same path,
so commands triggered schema work before their handlers ran.

Now `connect_postgres_db` only establishes the SeaORM connection. The explicit
`database::sync_schema` operation owns model sync and manual index handling.
`database::connect()` still migrates the library-owned Tower Sessions schema.

The failure was triggered by the custom unique index `idx-task_runs-backfill_task`,
which is created outside the model registry. SeaORM 2.0.4 treats unmatched
unique indexes as obsolete and its PostgreSQL cleanup branch uses
`ALTER TABLE ... DROP CONSTRAINT`, invalid for this standalone `CREATE UNIQUE
INDEX`. The explicit sync transaction now drops only this registered
application-managed unique index before model sync and recreates all registered
manual indexes afterward.

The admin command connects without running model sync or the session-store
migration. Keep Tower Sessions `PostgresStore::migrate()` separate; decide
independently whether it should remain on service startup.

## Implemented design

### 1. Make ordinary database connections non-mutating

- Separate connection establishment from application schema synchronization.
- Remove model registry sync and manual index DDL from normal web-service
  startup.
- Ensure normal admin commands also use a connection path that does not sync
  the application schema as a side effect.
- Keep session-store connection/migration behavior explicit and separate from
  the application-model schema path.

### 2. Centralize explicit schema ownership

`src/database/sync_schema.rs` exposes `database::sync_schema`, the sole
application-schema mutation path. It runs the following sequence in one
PostgreSQL transaction:

1. Acquire a transaction-level advisory lock so overlapping runs serialize.
2. Drop only registered manual unique indexes that SeaORM 2.0.4 misclassifies.
   Never drop all database indexes, primary keys, or indexes backing
   model-declared constraints.
3. Run the additive SeaORM model registry sync.
4. Recreate/ensure every registered application-managed index, including the non-unique inference run/attempt index.
5. Upsert `dreamscroll_schema_version` with `REQUIRED_SCHEMA_VERSION` (currently 6).

The application requires a minimum schema version. `database::connect` checks
the marker before migrating the Tower Sessions store or starting the web app.
Missing or stale markers fail with an instruction to run
`dreamscroll_admin sync_schema`; a newer marker is accepted for compatible
rolling deployments. Bump `REQUIRED_SCHEMA_VERSION` with every model or manual
index change that requires a schema sync. The marker is an operator/version
contract, not a catalog diff; untracked manual DB edits are not detected.
The marker is a one-row `dreamscroll_schema_version` table, initially at version
1. Sync creates or updates it only after model sync and index creation succeed,
within the same transaction.
Sync refuses to run when an existing marker is newer than the current binary's
required version, preventing a stale admin image from reconciling or downgrading
a newer schema. Web startup still accepts newer markers for compatible rollouts.

Schema version 3 adds nullable `illuminations.inference_meta_id`. Existing
projection content is retained with a NULL link; future projection writes update
the link in the same transaction as content. Raw results keep their independent
link, since raw and projection writes need not succeed together.

Schema version 5 names the non-null JSONB provider-details column
`inference_metas.provider_metadata`, with an empty-object default. New inference
rows separate request-only `params` (including `prompt_version`) from provider
usage, IDs, and grounding in `provider_metadata`. Run explicit schema sync; no
table drops are needed for the version-3 or version-4 upgrade. Historical `params`
values are not rewritten, and data from the earlier version-4 column is not
transferred. If that obsolete column exists, its removal remains manual.

Schema version 6 adds nullable BIGINT `inference_metas.tokens_thought` for
provider-reported thinking/reasoning usage. Existing rows retain NULL; counts are
not inferred or backfilled from historical usage JSON. Run explicit schema sync
before starting the updated application.

The transaction prevents other sessions from observing the temporary absence of
the custom unique index and rolls back partial DDL on failure. It still holds
locks while rebuilding indexes; measure this on a populated database and use a
controlled window if lock duration is material. Do not use `CREATE/DROP INDEX
CONCURRENTLY` inside the transaction.

`src/database/indexes/task_run.rs` owns the manual index definitions and
whether each must be removed before sync. SeaORM 2.0.4 inspects unmatched
**unique** indexes; the ordinary `idx-task_runs-backfill_status` index does not
trigger this specific drop path. Keep future hand-written indexes in this
registry so their lifecycle remains explicit.
The pre-sync drop qualifies the index with `current_schema()` so a matching name
in a later `search_path` schema cannot be removed accidentally.

This explicit command will rebuild the registered custom unique index each time
it is run with SeaORM 2.0.4. That is acceptable only for an infrequent operator
operation and must be measured; it is not acceptable to restore this operation
to service startup. Revisit this workaround when SeaORM's custom table-level
index support is released (see PR #3231 below), or move to explicit index
migrations if rebuild cost becomes material.

`dreamscroll_admin sync_schema` runs the explicit transaction above and exits
non-zero on error. On success, the version marker is committed atomically with
the model schema and manual indexes.

Normal service startup never applies application-model schema changes. It only
checks that the applied marker meets the current binary's minimum requirement.
This command is not a dry-run or an ordered migration history; it reconciles the
current entities with the live schema and records one compatible version.

### 4. Keep the limitation of model sync explicit

SeaORM's Entity-first `SchemaBuilder::sync` is additive: it creates missing tables,
columns, keys, and foreign keys, but does not drop retired tables/columns or alter
existing column types. It is not a versioned migration ledger. The operator
manually drops obsolete inference-metadata tables and removes old bookkeeping
columns from `illumination_raws` (or drops/recreates affected prototype tables)
before sync. No inference-specific destructive cleanup is embedded in the command;
no historical data transfer is needed.
It also has special unique-index drop behavior, as demonstrated above.

**Formal SeaORM option:** the official `sea-orm-migration` crate provides
ordered `MigrationTrait::up` / `down` migrations, records applied migration
names in `seaql_migrations`, supports SeaQuery or raw SQL, and can run data
updates. It does not automatically apply the latest entity definitions. Moving
an already-existing production database to it requires a baseline/adoption plan;
new migrations then need explicit DDL and data-change code. Use this route before
non-additive changes, data backfills, or rollouts requiring staged
expand/contract compatibility. Revisit the dependency decision at that point;
do not quietly combine versioned migrations and model-sync into two competing
schema authorities.

For this first cut, stay with the requested explicit model-sync command and no
new dependency. If the operation proves too risky or expensive on production
data, stop extending it and adopt the official migrator with a deliberate
baseline rather than adding more sync workarounds.

## Production execution on Google Cloud

Read-only inspection on 2026-10-16 confirmed the production service is
`dreamscroll-webui-api` in project `mdrcode`, region `us-central1`. It connects
to Cloud SQL instance `mdrcode:us-central1:dreamscroll-prod-db4`, uses
`POSTGRES_HOST_PORT=localhost` with
`POSTGRES_CONNECTION_PARAMS=host=/cloudsql/mdrcode:us-central1:dreamscroll-prod-db4`,
and gets `POSTGRES_PASSWORD` from Secret Manager. Its Cloud Run configuration
also has Direct VPC networking. No Cloud Run Job was listed in that region.
No cloud resources or database contents were changed during this inspection.

Production plan:

1. The root `Dockerfile` now builds and packages `dreamscroll_admin` alongside
   `dreamscroll_web`; Cloud Run Job configuration and execution are deferred.
2. Create a single-task Cloud Run Job with command
   `/app/dreamscroll_admin` and argument `sync_schema`. Attach the same Cloud SQL
   instance and networking required by the production service. Configure the
   DB password as a Secret Manager reference; never pass it as a CLI argument or
   plain build substitution.
3. Give the job identity only the needed Secret Manager access and Cloud SQL
   connection permissions; confirm the database user itself has the required
   DDL privileges. The current service uses the default Compute Engine service
   account; evaluate a dedicated migration identity rather than inheriting
   unrelated service permissions by default.
4. Run one task with no automatic retry and a bounded timeout. Execute manually
   with `gcloud run jobs execute <job> --region=us-central1 --wait`, then check
   Cloud Run execution status and Cloud Logging before deploying application
   code that depends on the schema change.
5. Use the same immutable commit-tagged image for the schema job and web
   service. The current `gcloud/cloudbuild.yaml` deploys the service as part of
   its build/push flow; split or gate that flow so an operator can run and
   verify the schema job before the service deployment. Do not auto-execute the
   schema job on every image build.

`dreamscroll_admin` builds a `PostgresConfig` from only the `POSTGRES_*`
environment variables for database-backed commands. `enums` and `hash_password`
do not construct a config or connect to Postgres. The Cloud Run schema job needs
no Gemini, storage, or task-queue credentials; supply only the required Postgres
settings, with the password from Secret Manager.

For changes incompatible with the currently serving revision, use an
expand/contract rollout: make the schema compatible with both revisions first,
run the schema operation, deploy compatible application code, and remove old
schema only in a later explicit step. Confirm an available staging target and
production backup/restore procedure before the first production run.

## Implementation and rollout checklist

1. [x] Inventory application entities, model-declared constraints, manual
   indexes, and session-store schema behavior.
2. [ ] Confirm the live production schema and database user's DDL privileges
   before the first production run.
3. [x] Remove application model sync from normal connections and centralize
   explicit schema synchronization and manual index ownership.
4. [x] Add `dreamscroll_admin sync_schema`, Postgres regression tests for
   repeated sync/index uniqueness, and startup rejection of missing/stale
   markers while accepting a newer compatible marker.
5. [ ] Measure index rebuild/lock duration on a representative production-sized
   table. Verify rollback behavior on a deliberate sync failure.
6. [ ] Verify the Docker image and admin command in a non-production Cloud Run
   Job. Cloud Run Job configuration is intentionally deferred for now.
7. [ ] Split or gate the release sequence so production schema sync succeeds
   before deploying application code that depends on the change.
8. [ ] Record production backup and rollback steps. `SchemaBuilder::sync` is not
   a reversible migration; reverting the app image does not restore schema.

## Research sources

- [SeaORM Entity-first workflow](https://www.sea-ql.org/SeaORM/docs/generate-entity/entity-first/)
  — schema registry sync, its index exception, and full schema discovery.
- [SeaORM migration setup](https://www.sea-ql.org/SeaORM/docs/migration/setting-up-migration/)
  — official migration crate and `seaql_migrations` tracking table.
- [SeaORM writing migrations](https://www.sea-ql.org/SeaORM/docs/migration/writing-migration/)
  — explicit DDL and data changes with `up` / `down`.
- [SeaORM running migrations](https://www.sea-ql.org/SeaORM/docs/migration/running-migration/)
  — CLI, programmatic `Migrator::up`, status and rollback operations.
- [SeaORM issue #3157](https://github.com/SeaQL/sea-orm/issues/3157)
  — known issue that custom indexes may be dropped by a later schema sync.
- [SeaORM PR #3231](https://github.com/SeaQL/sea-orm/pull/3231)
  — proposed table-level index declarations integrated with schema sync; open
  as of this plan date, not a released fix.
- [Cloud Run Jobs](https://docs.cloud.google.com/run/docs/create-jobs)
  — one-off job configuration, Cloud SQL/networking, task retries and timeout.
- [Run a Cloud Run Job](https://docs.cloud.google.com/run/docs/execute/jobs)
  — manual execution, `--wait`, execution status and logs.
- [Configure job containers](https://docs.cloud.google.com/run/docs/configuring/jobs/containers)
  — entrypoint and argument configuration.
- [Configure job secrets](https://docs.cloud.google.com/run/docs/configuring/jobs/secrets)
  — Secret Manager references and service identity permissions.
- [Connect Cloud Run to Cloud SQL for PostgreSQL](https://docs.cloud.google.com/sql/docs/postgres/connect-run)
  — Cloud SQL attachment, socket, networking and IAM requirements.
- [`gcloud run jobs execute`](https://docs.cloud.google.com/sdk/gcloud/reference/run/jobs/execute)
  — `--wait`, execution argument overrides and required CLI semantics.