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

pub async fn before_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(BACKFILL_ALT_SEARCH_QUERIES).await?;
    Ok(())
}

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(CREATE).await?;
    conn.execute_unprepared(ARRAY_DEFAULTS).await?;
    conn.execute_unprepared(NOT_SELF_MERGED).await?;
    Ok(())
}
