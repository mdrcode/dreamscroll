use sea_orm::{ConnectionTrait, DbErr};

/// The registry creates array columns as `NOT NULL` with no default, so an insert that omits them
/// fails instead of starting empty. Every entity insert legitimately omits them — the accumulated
/// text is what `project` fills — so the default belongs in the schema.
const ARRAY_DEFAULTS: &str = r#"
ALTER TABLE knowledge_entities ALTER COLUMN alt_names SET DEFAULT '{}';
ALTER TABLE knowledge_entities ALTER COLUMN alt_types SET DEFAULT '{}';
ALTER TABLE knowledge_entities ALTER COLUMN alt_descriptions SET DEFAULT '{}';
ALTER TABLE knowledge_entities ALTER COLUMN alt_search_queries SET DEFAULT '{}';
ALTER TABLE knowledge_identities ALTER COLUMN classes SET DEFAULT '{}';
ALTER TABLE knowledge_blocking_index ALTER COLUMN word_keys SET DEFAULT '{}';
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

/// One row per identity, ever. `(scheme, external_id)` is what an identity is.
const IDENTITIES_UNIQUE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-knowledge_identities_identity"
    ON knowledge_identities (scheme, external_id)
"#;

/// An entity asserts an identity once. Retracting frees the slot, so the claim is a fact with a
/// timestamp rather than a status.
const CLAIMS_UNIQUE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-knowledge_claims_entity_identity"
    ON knowledge_claims (knowledge_entity_id, knowledge_identity_id)
    WHERE retracted_at IS NULL
"#;

/// The blocker's access path: which entities wear any of these folded surfaces. `array_ops` GIN is
/// what makes `surfaces && $1` one probe instead of a scan.
const INDEX_SURFACES: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-knowledge_blocking_index-surfaces"
    ON knowledge_blocking_index USING gin (surfaces);
CREATE INDEX IF NOT EXISTS "idx-knowledge_blocking_index-word_keys"
    ON knowledge_blocking_index USING gin (word_keys)
"#;

/// The pair, which is how the batch is read (the merge pass and the list). Not unique: the table is
/// one scan's output and is replaced wholesale, and `scan` itself raises a pair once even when two
/// identities connect the same two entities.
const PROXIMITIES_PAIR: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-knowledge_proximities_pair"
    ON knowledge_proximities (holder_id, counterpart_id)
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
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_identities_identity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_claims_entity_identity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_proximities_pair');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_blocking_index-surfaces');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_blocking_index-word_keys');
END;
$sync$;
"#;

/// Schema 24 renamed the proximity basis from `shared_identity` to `shared_id`. Runs *after* the
/// unique index is dropped: two rows that differed only by basis collide once renamed, which is how a
/// stale writer's rows showed up as duplicates. The dedupe keeps the oldest of an identical pair.
const MIGRATE_PROXIMITY_BASIS: &str = r#"
DO $sync$
BEGIN
    IF to_regclass('knowledge_proximities') IS NOT NULL THEN
        UPDATE knowledge_proximities SET basis = 'shared_id' WHERE basis = 'shared_identity';
        DELETE FROM knowledge_proximities a USING knowledge_proximities b
         WHERE a.id > b.id AND a.holder_id = b.holder_id AND a.counterpart_id = b.counterpart_id
           AND a.basis = b.basis
           AND coalesce(a.knowledge_identity_id, 0) = coalesce(b.knowledge_identity_id, 0);
    END IF;
END;
$sync$;
"#;

/// A claim records what the *question* was about (`basis`), which the projection counts instead of
/// guessing from the provider's name. No backfill: every existing claim predates the fact, the column
/// is `NOT NULL`, and the table regenerates from `link` + `resolve` — so drop it and let the registry
/// create it.
const RETIRE_CLAIMS_WITHOUT_BASIS: &str = r#"
DO $sync$
BEGIN
    IF to_regclass('knowledge_claims') IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM information_schema.columns
            WHERE table_schema = current_schema()
              AND table_name = 'knowledge_claims'
              AND column_name = 'basis')
    THEN
        DROP TABLE knowledge_claims;
    END IF;
END;
$sync$;
"#;

pub async fn before_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(RETIRE_CLAIMS_WITHOUT_BASIS).await?;
    conn.execute_unprepared(BACKFILL_ALT_SEARCH_QUERIES).await?;
    conn.execute_unprepared(DROP_BEFORE_SYNC).await?;
    conn.execute_unprepared(MIGRATE_PROXIMITY_BASIS).await?;
    Ok(())
}

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(IDENTITIES_UNIQUE).await?;
    conn.execute_unprepared(CLAIMS_UNIQUE).await?;
    conn.execute_unprepared(PROXIMITIES_PAIR).await?;
    conn.execute_unprepared("DROP INDEX IF EXISTS \"idx-knowledge_proximities_open\"")
        .await?;
    conn.execute_unprepared(INDEX_SURFACES).await?;
    conn.execute_unprepared(ARRAY_DEFAULTS).await?;
    conn.execute_unprepared(NOT_SELF_MERGED).await?;
    Ok(())
}
