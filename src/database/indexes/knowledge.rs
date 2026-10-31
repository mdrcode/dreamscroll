use sea_orm::{ConnectionTrait, DbErr};

/// The identity invariant, enforced where the claims are.
///
/// SeaORM's derive cannot express an expression index with a predicate, so it is manual: no two
/// cores may hold a *live* identity claim to the same `(scheme, id)`. Retracting a claim
/// (`retracted_at`) frees the slot, which is why the predicate is part of the index rather than a
/// separate table — append-only evidence can only be unique while a claim is believed.
const CREATE: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS "idx-core_evidence_identity"
    ON core_knowledge_evidence ((value->>'scheme'), (value->>'id'))
    WHERE kind IN ('ontology', 'platform_link') AND retracted_at IS NULL
"#;

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(CREATE).await?;
    Ok(())
}
