use sea_orm::{ConnectionTrait, DbErr};

/// The identity invariant, enforced where the claims are.
///
/// SeaORM's derive cannot express an expression index with a predicate, so it is manual: no two
/// knowledge entities may hold a *live* claim to the same `(scheme, id)`. Retracting a claim
/// (`retracted_at`) frees the slot, which is why the predicate is part of the index rather than a
/// separate table — append-only evidence can only be unique while a claim is believed.
const CREATE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-knowledge_evidence_identity"
    ON knowledge_evidence ((value->>'scheme'), (value->>'id'))
    WHERE kind = 'claim' AND retracted_at IS NULL
"#;

/// The registry creates array columns as `NOT NULL` with no default, so an insert that omits them
/// fails instead of starting empty. Every entity insert legitimately omits them — the accumulated
/// text is what `project` fills — so the default belongs in the schema.
const ARRAY_DEFAULTS: &str = r#"
ALTER TABLE knowledge_entities ALTER COLUMN alt_names SET DEFAULT '{}';
ALTER TABLE knowledge_entities ALTER COLUMN alt_types SET DEFAULT '{}';
ALTER TABLE knowledge_entities ALTER COLUMN alt_descriptions SET DEFAULT '{}';
ALTER TABLE knowledge_entities ALTER COLUMN alt_search_queries SET DEFAULT '{}';
ALTER TABLE knowledge_claims ALTER COLUMN classes SET DEFAULT '{}';
"#;

/// A `NOT NULL` array column cannot be added to a populated table without a default, and the
/// registry creates one without: every existing entity needs its empty collection first.
///
/// Guarded on the table existing, because on a fresh database the registry has not created it yet.
const BACKFILL_ALT_SEARCH_QUERIES: &str = r#"
DO $sync$
BEGIN
    IF to_regclass('knowledge_entities') IS NOT NULL THEN
        ALTER TABLE knowledge_entities
            ADD COLUMN IF NOT EXISTS alt_search_queries text[] NOT NULL DEFAULT '{}';
    END IF;
END;
$sync$;
"#;

/// The identity invariant, and the only one of the three that lives in `knowledge_claims`.
///
/// No two entities may hold a *live* claim to the same `(scheme, external_id)`. Retracting a claim
/// (`retracted_at`) frees the slot, which is why the predicate is part of the index rather than a
/// separate table — claims can only be unique while they are believed.
const CLAIMS_UNIQUE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-knowledge_claims_identity"
    ON knowledge_claims (scheme, external_id)
    WHERE retracted_at IS NULL
"#;

/// One live record per contested pair and identity: the queue is a set, and a second `resolve` run
/// re-asserting the same contradiction must add nothing.
const COLLISIONS_UNIQUE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-knowledge_collisions_pair"
    ON knowledge_collisions (holder_id, claimant_id, scheme, external_id)
    WHERE adjudicated_at IS NULL
"#;

/// A tombstone must point at another entity, never at itself: self-merge is a no-op in the code, and
/// a row that pointed at itself would be an unbreakable redirect loop.
///
/// Only self-reference fits a `CHECK`. A longer cycle (A → B → A) is not expressible here; the reader
/// that follows pointers bounds its hops and fails loudly instead.
const NOT_SELF_MERGED: &str = r#"
DO $sync$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'knowledge_entities_merged_into_not_self'
    ) THEN
        ALTER TABLE knowledge_entities
            ADD CONSTRAINT knowledge_entities_merged_into_not_self
            CHECK (merged_into_id IS NULL OR merged_into_id <> id);
    END IF;
END;
$sync$;
"#;

/// SeaORM 2.0.4 misclassifies a custom unique index as a constraint and tries to drop it during sync
/// — then fails, because no constraint of that name exists. Removing them first is the same
/// workaround `indexes::task_run` uses.
const DROP_BEFORE_SYNC: &str = r#"
DO $sync$
BEGIN
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_claims_identity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_collisions_pair');
END;
$sync$;
"#;

pub async fn before_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(BACKFILL_ALT_SEARCH_QUERIES).await?;
    conn.execute_unprepared(DROP_BEFORE_SYNC).await?;
    Ok(())
}

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(CREATE).await?;
    conn.execute_unprepared(CLAIMS_UNIQUE).await?;
    conn.execute_unprepared(COLLISIONS_UNIQUE).await?;
    conn.execute_unprepared(ARRAY_DEFAULTS).await?;
    conn.execute_unprepared(NOT_SELF_MERGED).await?;
    Ok(())
}
