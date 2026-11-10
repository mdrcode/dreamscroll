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
    /// `merge`, `veto_identity` or `distinct`.
    pub kind: String,
    pub winner: String,
    pub loser: String,
    /// The entity a veto is about, by name now or by origin if it no longer exists.
    pub subject: String,
    /// `scheme:external_id` a veto excludes.
    pub identity: String,
    pub note: String,
    pub decided_at: DateTime<Utc>,
    /// The stable keys: origin observations, not generation-scoped entity ids.
    pub winner_origin: Option<i32>,
    pub loser_origin: Option<i32>,
    pub subject_origin: Option<i32>,
}

impl VerdictRow {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "generation": self.generation,
            "cause": self.cause,
            "kind": self.kind,
            "winner": self.winner,
            "loser": self.loser,
            "subject": self.subject,
            "identity": self.identity,
            "note": self.note,
            "decided_at": self.decided_at.to_rfc3339(),
            "winner_origin": self.winner_origin,
            "loser_origin": self.loser_origin,
            "subject_origin": self.subject_origin,
        })
    }
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
                    coalesce(v.note, ''), v.decided_at, \
                    v.winner_origin, v.loser_origin, v.subject_origin \
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
                winner_origin: row.try_get_by_index(10)?,
                loser_origin: row.try_get_by_index(11)?,
                subject_origin: row.try_get_by_index(12)?,
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
        for (generation, cause) in [(1, crate::model::knowledge_verdicts::Cause::Repair), (2, crate::model::knowledge_verdicts::Cause::Repair), (2, crate::model::knowledge_verdicts::Cause::ProximityAuto)] {
            crate::model::knowledge_verdicts::Entity::insert(
                crate::model::knowledge_verdicts::ActiveModel {
                    generation: Set(generation),
                    cause: Set(cause),
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
