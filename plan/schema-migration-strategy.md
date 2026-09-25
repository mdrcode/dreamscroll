# Schema migration strategy

**Status:** exploratory. **Date:** 2026-09-25.

## Current issue

Dreamscroll currently runs SeaORM schema synchronization during every service
startup:

```rust
conn.get_schema_registry("dreamscroll::model::*")
    .sync(&conn)
    .await?;
```

This is convenient for local development and prototype iteration, but SeaORM's
schema synchronizer currently reissues ordinary index creation statements. For
example, the `#[sea_orm(indexed)]` annotation on
`task_run_status.entity_id` generated a `CREATE INDEX` for the already-existing
`idx-task_run_status-entity_id` index. PostgreSQL skipped the duplicate, but
startup spent about 2.5 seconds in schema synchronization and emitted a notice.

For now, the index annotation has been removed from the model. The existing
database index is intentionally left in place. This prevents SeaORM from
attempting to recreate it while retaining the current local database behavior.
The annotation should be restored once schema synchronization handles existing
ordinary indexes without redundant DDL.

## Perspective

Schema synchronization is useful during rapid prototype development because it
automatically creates newly added tables and columns. It is not a complete
migration system:

- it runs on every startup;
- it is non-destructive and does not safely express all schema changes;
- model metadata and live database state can drift;
- index handling is currently noisy and unnecessarily slow;
- startup latency becomes coupled to database catalog discovery and DDL checks.

The current approach is acceptable for local development, but should not be the
long-term production schema strategy.

## Potential direction

Move toward explicit, versioned migrations for production and normal startup.
The migration path should own:

- table and column creation;
- indexes, including composite indexes;
- unique constraints and foreign keys;
- intentional schema changes and data backfills.

Schema synchronization can remain available behind a local-development setting
for quickly bootstrapping a fresh database, but normal startup should skip it
once the database has been initialized. Indexes should be created explicitly
with migration-safe DDL such as `CREATE INDEX IF NOT EXISTS` where appropriate.

## Follow-up investigation

1. Inventory all current entities, indexes, constraints, and session-store
   tables.
2. Determine whether to use SeaORM migrations or a small project-specific
   migration runner.
3. Add a configuration switch separating bootstrap/schema development from
   normal application startup.
4. Measure startup with schema synchronization disabled after initialization.
5. Restore model-level index annotations only after the chosen migration or
   synchronization strategy makes their behavior predictable.