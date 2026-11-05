//! The modification log as a person reads it: what was folded into what, and what decided it.
//!
//! A report over the table rather than graph work, so it lives here with the other reports.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::database;

/// One logged merge.
pub struct VerdictRow {
    pub id: i32,
    pub generation: i32,
    /// `repair`, `proximity_auto` or `replayed`.
    pub cause: String,
    pub winner: String,
    pub loser: String,
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
            "SELECT id, generation, cause, \
                    coalesce(winner_label, ''), coalesce(loser_label, ''), decided_at \
               FROM knowledge_verdicts \
              WHERE ($1::int IS NULL OR generation = $1) \
                AND ($2::text IS NULL OR cause = $2) \
              ORDER BY id DESC \
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
                winner: row.try_get_by_index(3)?,
                loser: row.try_get_by_index(4)?,
                decided_at: row.try_get_by_index(5)?,
            })
        })
        .collect()
}
