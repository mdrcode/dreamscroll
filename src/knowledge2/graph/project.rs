//! The one derived write: an entity's substantiation follows its identity rows.
//!
//! Name, type, description and platform presence are set when the entity is created from its
//! origin; membership does not change them today. What every identity mutation changes is `status`
//! and `confidence`, and both are a pure function of the rows in `knowledge_identities`.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};

/// Recompute the identity-dependent columns. Inside the caller's transaction, always.
pub(super) async fn refresh<C: ConnectionTrait>(conn: &C, entity: i32) -> anyhow::Result<()> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
        UPDATE knowledge_entities e
           SET status = CASE
                   WHEN EXISTS (
                       SELECT 1 FROM knowledge_identities i WHERE i.knowledge_entity_id = e.id
                   ) THEN 'identified'
                   ELSE 'provisional'
               END,
               confidence = (
                   SELECT max(i.confidence)
                     FROM knowledge_identities i
                    WHERE i.knowledge_entity_id = e.id
               ),
               updated_at = now()
         WHERE e.id = $1
           AND e.merged_into_id IS NULL
        "#,
        vec![Value::Int(Some(entity))],
    ))
    .await?;
    Ok(())
}
