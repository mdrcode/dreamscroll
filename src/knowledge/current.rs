//! Which reading of a capture the user currently sees.
//!
//! The graph holds **every** reading of every capture — an illumination can be wrong, and the
//! correct reading is often the older one, so `link` no longer discards history. The user's view is
//! the opposite: it starts from the capture's *current* illumination and left-joins into knowledge
//! entities, so a reading the capture no longer stands behind can never surface.
//!
//! Current means the highest illumination id for the capture. Illuminations are only ever appended,
//! so a higher id is a later run. Nothing stores the flag; deriving it means it cannot drift.
//!
//! See `plan/knowledge-illumination-history.md`.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

/// The illuminations this user's captures currently stand behind, one per capture.
pub async fn illumination_ids<C: ConnectionTrait>(
    conn: &C,
    user_id: i32,
) -> anyhow::Result<Vec<i32>> {
    // `DISTINCT ON` is the whole point of the query and SeaORM cannot express it.
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT DISTINCT ON (capture_id) id FROM illuminations \
             WHERE user_id = $1 ORDER BY capture_id, id DESC",
            [user_id.into()],
        ))
        .await?;
    rows.into_iter()
        .map(|row| row.try_get_by_index::<i32>(0).map_err(Into::into))
        .collect()
}
