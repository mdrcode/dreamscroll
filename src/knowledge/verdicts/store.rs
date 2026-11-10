//! The store: one connection, the merge log's append, and the key decisions share.

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
};

use crate::database::DbHandle;
use crate::knowledge::graph::current_generation;
use crate::knowledge::graph::merge::Cause;
use crate::model;
use crate::model::knowledge_verdicts::Kind;

/// Every write to `knowledge_verdicts`, and the reads a decision needs.
///
/// Holds its own clone of the connection, like `Evidence`. The one method that does not use it is
/// `record`: a merge's log row must commit with the fold, so `merge` hands it the transaction.
#[derive(Clone)]
pub struct VerdictStore {
    pub(super) conn: DatabaseConnection,
}

impl VerdictStore {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    pub fn from_handle(db: &DbHandle) -> Self {
        Self::new(db.conn.clone())
    }

    /// Append a merge row, on the connection the caller passes.
    ///
    /// `merge` calls this with the merge transaction, so the entry commits or rolls back with the
    /// fold. Idempotent per cause, generation and direction, so replaying a repair does not grow the
    /// log and an automatic fold is only ever noted once in the generation it happened in. A row
    /// without an origin cannot be named after a rebuild, so such a merge does not enter the log.
    pub(crate) async fn record<C: ConnectionTrait>(
        &self,
        conn: &C,
        cause: Cause,
        winner: &model::knowledge_entity::Model,
        loser: &model::knowledge_entity::Model,
    ) -> anyhow::Result<()> {
        let (Some(winner_origin), Some(loser_origin)) =
            (winner.origin_observation_id, loser.origin_observation_id)
        else {
            return Ok(());
        };
        let generation = current_generation(conn).await?;
        let existing = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Cause.eq(cause.as_str()))
            .filter(model::knowledge_verdicts::Column::Generation.eq(generation))
            .filter(model::knowledge_verdicts::Column::WinnerOrigin.eq(winner_origin))
            .filter(model::knowledge_verdicts::Column::LoserOrigin.eq(loser_origin))
            .one(conn)
            .await?;
        if existing.is_some() {
            return Ok(());
        }
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(generation),
            cause: Set(cause),
            kind: Set(Kind::Merge),
            winner_origin: Set(Some(winner_origin)),
            loser_origin: Set(Some(loser_origin)),
            winner_label: Set(Some(winner.name.clone())),
            loser_label: Set(Some(loser.name.clone())),
            ..Default::default()
        })
        .exec(conn)
        .await?;
        Ok(())
    }
}

/// The unordered pair two origins name, as a key.
pub(crate) fn pair_key(a: i32, b: i32) -> (i32, i32) {
    (a.min(b), a.max(b))
}
