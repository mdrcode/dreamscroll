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
"#;

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(CREATE).await?;
    conn.execute_unprepared(ARRAY_DEFAULTS).await?;
    Ok(())
}
