//! The modification log as a person reads it: what was folded into what, and what decided it.
//!
//! A report over the table rather than graph work, so it lives here with the other reports.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::database;

/// One logged decision: a merge (two entities) or a veto (one entity, one identity).
#[derive(Debug)]
pub struct VerdictRow {
    pub id: i32,
    pub generation: i32,
    /// `repair`, `proximity_auto` or `replayed`.
    pub cause: String,
    /// `merge` or `veto`.
    pub kind: String,
    pub winner: String,
    pub loser: String,
    /// The entity a veto is about, by name now or by origin if it no longer exists.
    pub subject: String,
    /// `scheme:external_id` a veto excludes.
    pub identity: String,
    pub note: String,
    pub decided_at: DateTime<Utc>,
}

/// Newest first: an audit is read from the present backwards.
pub async fn verdict_list(
    db: &database::DbHandle,
    generation: Option<i32>,
    cause: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<VerdictRow>> {
    let rows = db
        .conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT v.id, v.generation, v.cause, v.kind, \
                    coalesce(v.winner_label, ''), coalesce(v.loser_label, ''), \
                    coalesce(e.name, 'origin ' || v.subject_origin::text, ''), \
                    coalesce(v.identity_scheme || ':' || v.identity_external_id, ''), \
                    coalesce(v.note, ''), v.decided_at \
               FROM knowledge_verdicts v \
               LEFT JOIN knowledge_entities e ON e.origin_observation_id = v.subject_origin \
              WHERE ($1::int IS NULL OR v.generation = $1) \
                AND ($2::text IS NULL OR v.cause = $2) \
              ORDER BY v.id DESC \
              LIMIT $3",
            [
                generation.into(),
                cause.map(str::to_string).into(),
                (limit as i64).into(),
            ],
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok(VerdictRow {
                id: row.try_get_by_index(0)?,
                generation: row.try_get_by_index(1)?,
                cause: row.try_get_by_index(2)?,
                kind: row.try_get_by_index(3)?,
                winner: row.try_get_by_index(4)?,
                loser: row.try_get_by_index(5)?,
                subject: row.try_get_by_index(6)?,
                identity: row.try_get_by_index(7)?,
                note: row.try_get_by_index(8)?,
                decided_at: row.try_get_by_index(9)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use sea_orm::{EntityTrait, Set};

    use super::*;
    use crate::test_support::test_db;

    #[tokio::test]
    async fn the_log_is_filtered_by_generation_and_cause_and_read_newest_first() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        for (generation, cause) in [(1, "repair"), (2, "repair"), (2, "proximity_auto")] {
            crate::model::knowledge_verdicts::Entity::insert(
                crate::model::knowledge_verdicts::ActiveModel {
                    generation: Set(generation),
                    cause: Set(cause.to_string()),
                    winner_origin: Set(Some(generation)),
                    loser_origin: Set(Some(generation + 100)),
                    winner_label: Set(Some("winner".to_string())),
                    loser_label: Set(Some("loser".to_string())),
                    ..Default::default()
                },
            )
            .exec(&db.conn)
            .await
            .unwrap();
        }

        let all = verdict_list(&db, None, None, 50).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].cause, "proximity_auto", "newest first");

        assert_eq!(verdict_list(&db, Some(2), None, 50).await.unwrap().len(), 2);
        let repairs = verdict_list(&db, Some(2), Some("repair"), 50).await.unwrap();
        assert_eq!(repairs.len(), 1, "{repairs:?}");
        assert_eq!(repairs[0].winner, "winner");
        assert_eq!(
            verdict_list(&db, None, None, 2).await.unwrap().len(),
            2,
            "the limit applies after the filters"
        );
    }
}
