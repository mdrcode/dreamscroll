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

/// One provider annotates one entity with one key once. The provider is part of the key so two
/// authorities may corroborate one identity without overwriting each other.
const IDENTITIES_UNIQUE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-knowledge_identities_entity_provider_identity"
    ON knowledge_identities (knowledge_entity_id, provider, scheme, external_id)
"#;

/// The durable key is looked up across entities: `query_entities_for_identity`, the proximity scan,
/// and a veto from the identity side.
const IDENTITIES_KEY: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-knowledge_identities_identity"
    ON knowledge_identities (scheme, external_id)
"#;

/// Selection asks whether one provider has annotated a set of entities.
const IDENTITIES_PROVIDER: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-knowledge_identities_provider_entity"
    ON knowledge_identities (provider, knowledge_entity_id)
"#;

/// Stats count observations against the annotation that carried them.
const IDENTITIES_OBSERVATION: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-knowledge_identities_observation"
    ON knowledge_identities (user_observed_entity_id)
"#;

/// Projection reads one entity's members; without this it scans every observation.
const OBSERVATIONS_ENTITY: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-user_observed_entities-knowledge_entity"
    ON user_observed_entities (knowledge_entity_id)
"#;

/// The blocker's access path: which entities wear any of these folded surfaces. `array_ops` GIN is
/// what makes `surfaces && $1` one probe instead of a scan.
const INDEX_SURFACES: &str = r#"
CREATE INDEX IF NOT EXISTS "idx-knowledge_blocking_index-surfaces"
    ON knowledge_blocking_index USING gin (surfaces);
CREATE INDEX IF NOT EXISTS "idx-knowledge_blocking_index-word_keys"
    ON knowledge_blocking_index USING gin (word_keys)
"#;

/// An identity's durable name cannot be empty: `('', '')` is not a thing the world names.
const IDENTIFIER_NOT_EMPTY: &str = r#"
DO $sync$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'knowledge_identities_identifier_not_empty'
    ) THEN
        ALTER TABLE knowledge_identities
            ADD CONSTRAINT knowledge_identities_identifier_not_empty
            CHECK (scheme <> '' AND external_id <> '');
    END IF;
END;
$sync$;
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
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_identities_entity_provider_identity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_identities_identity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_identities_provider_entity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_identities_observation');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-user_observed_entities-knowledge_entity');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_proximities_pair');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_blocking_index-surfaces');
    EXECUTE format('DROP INDEX IF EXISTS %I.%I', current_schema(), 'idx-knowledge_blocking_index-word_keys');
END;
$sync$;
"#;

pub async fn before_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(BACKFILL_ALT_SEARCH_QUERIES).await?;
    conn.execute_unprepared(DROP_BEFORE_SYNC).await?;
    Ok(())
}

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(IDENTITIES_UNIQUE).await?;
    conn.execute_unprepared(IDENTITIES_KEY).await?;
    conn.execute_unprepared(IDENTITIES_PROVIDER).await?;
    conn.execute_unprepared(IDENTITIES_OBSERVATION).await?;
    conn.execute_unprepared(OBSERVATIONS_ENTITY).await?;
    conn.execute_unprepared(PROXIMITIES_PAIR).await?;
    conn.execute_unprepared("DROP INDEX IF EXISTS \"idx-knowledge_proximities_open\"")
        .await?;
    conn.execute_unprepared(INDEX_SURFACES).await?;
    conn.execute_unprepared(ARRAY_DEFAULTS).await?;
    conn.execute_unprepared(NOT_SELF_MERGED).await?;
    conn.execute_unprepared(IDENTIFIER_NOT_EMPTY).await?;
    Ok(())
}
