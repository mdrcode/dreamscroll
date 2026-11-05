//! Proximities as a person reads them: what each pair is, how close, what was decided.
//!
//! A report over the tables rather than graph work, so it lives here with the other reports.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::database;

/// One row per proximity, readable: what the pair is, how close, and what was decided.
///
/// The `note` says why a pair is worth a second look — `same entity_type`, or blank — and is also the
/// grouping. Ordered for triage, not by score: the highest scores are the pairs that must *never*
/// merge (same name, different type: Nintendo the organisation and the brand sit at 0.74), while the
/// actionable ones are same-type pairs whose names differ, where the shared identity is all the
/// evidence there is. Those come first, then score descending.
pub async fn proximity_list(
    db: &database::DbHandle,
    limit: usize,
) -> anyhow::Result<
    Vec<(
        i32,
        Option<f32>,
        String,
        String,
        String,
        String,
        String,
    )>,
> {
    let rows = db
        .conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT p.id, \
                    p.score, \
                    CASE WHEN h.entity_type = c.entity_type \
                         THEN 'same entity_type' ELSE '' END, \
                    p.basis, \
                    h.name || ' [' || h.entity_type || ']', \
                    c.name || ' [' || c.entity_type || ']', \
                    coalesce(i.scheme || ':' || i.external_id, '') \
               FROM knowledge_proximities p \
               JOIN knowledge_entities h ON h.id = p.holder_id \
               JOIN knowledge_entities c ON c.id = p.counterpart_id \
               LEFT JOIN knowledge_identities i ON i.id = p.knowledge_identity_id \
              ORDER BY (h.entity_type = c.entity_type) DESC, \
                       p.score DESC NULLS LAST, \
                       p.id \
              LIMIT $1",
            [(limit as i64).into()],
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get_by_index(0)?,
                row.try_get_by_index(1)?,
                row.try_get_by_index(2)?,
                row.try_get_by_index(3)?,
                row.try_get_by_index(4)?,
                row.try_get_by_index(5)?,
                row.try_get_by_index(6)?,
            ))
        })
        .collect()
}
