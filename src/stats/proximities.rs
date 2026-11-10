//! Proximities as a person reads them: what each pair is, how close, and whether a person has
//! already decided it.
//!
//! A report over the tables rather than graph work, so it lives here with the other reports.

use std::collections::BTreeSet;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

use crate::database;
use crate::model;

/// One candidate, readable.
pub struct ProximityRow {
    pub id: i32,
    pub score: Option<f32>,
    /// Why the pair is worth a second look — `same entity_type`, or blank.
    pub note: String,
    pub basis: String,
    /// The entity ids `merge` and `eval inspect` take; the names are for reading.
    pub holder_id: i32,
    pub counterpart_id: i32,
    pub holder: String,
    pub counterpart: String,
    pub identity: String,
    /// A person has already said these two are one thing.
    pub decided: bool,
}

/// One row per candidate. Ordered for triage, not by score: the highest scores are the pairs that
/// must *never* merge (same name, different type: Nintendo the organisation and the brand sit at
/// 0.74), while the actionable ones are same-type pairs whose names differ, where the shared identity
/// is all the evidence there is. Those come first, then score descending.
pub async fn proximity_list(
    db: &database::DbHandle,
    limit: usize,
) -> anyhow::Result<Vec<ProximityRow>> {
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
                    coalesce(i.scheme || ':' || i.external_id, ''), \
                    h.origin_observation_id, \
                    c.origin_observation_id, \
                    h.id, \
                    c.id \
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

    // A verdict names two origins; a candidate row names two entities. This is where the two meet,
    // and only for display — replay never asks about candidates.
    let decided: BTreeSet<(i32, i32)> = model::knowledge_verdicts::Entity::find()
        .all(&db.conn)
        .await?
        .into_iter()
        .filter_map(|verdict| {
            Some(crate::knowledge::verdicts::pair_key(
                verdict.winner_origin?,
                verdict.loser_origin?,
            ))
        })
        .collect();

    rows.iter()
        .map(|row| {
            let holder_origin: Option<i32> = row.try_get_by_index(7)?;
            let counterpart_origin: Option<i32> = row.try_get_by_index(8)?;
            let decided = match (holder_origin, counterpart_origin) {
                (Some(h), Some(c)) => decided
                    .contains(&crate::knowledge::verdicts::pair_key(h, c)),
                _ => false,
            };
            Ok(ProximityRow {
                id: row.try_get_by_index(0)?,
                score: row.try_get_by_index(1)?,
                note: row.try_get_by_index(2)?,
                basis: row.try_get_by_index(3)?,
                holder: row.try_get_by_index(4)?,
                counterpart: row.try_get_by_index(5)?,
                identity: row.try_get_by_index(6)?,
                holder_id: row.try_get_by_index(9)?,
                counterpart_id: row.try_get_by_index(10)?,
                decided,
            })
        })
        .collect()
}
