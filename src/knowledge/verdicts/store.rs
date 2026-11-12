//! The store: one connection, the merge log's append, and the key decisions share.

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};

use crate::database::DbHandle;
use crate::model;
use crate::model::knowledge_verdicts::Kind;

use super::Cause;

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
        generation: i32,
        cause: Cause,
        winner: &model::knowledge_entity::Model,
        loser: &model::knowledge_entity::Model,
        note: Option<&str>,
    ) -> anyhow::Result<()> {
        let (Some(winner_origin), Some(loser_origin)) =
            (winner.origin_observation_id, loser.origin_observation_id)
        else {
            return Ok(());
        };
        let existing = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Merge.as_str()))
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
            note: Set(note.map(str::to_string)),
            ..Default::default()
        })
        .exec(conn)
        .await?;
        Ok(())
    }

    /// Every merge a person recorded, oldest first.
    pub(crate) async fn merges(&self) -> anyhow::Result<Vec<model::knowledge_verdicts::Model>> {
        Ok(model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Merge.as_str()))
            .order_by_asc(model::knowledge_verdicts::Column::Id)
            .all(&self.conn)
            .await?)
    }

    /// Every veto a person recorded, oldest first.
    pub(crate) async fn vetoes(&self) -> anyhow::Result<Vec<model::knowledge_verdicts::Model>> {
        Ok(model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::VetoIdentity.as_str()))
            .order_by_asc(model::knowledge_verdicts::Column::Id)
            .all(&self.conn)
            .await?)
    }

    /// Whether a `distinct` verdict names this unordered pair of origins.
    ///
    /// The register's one definition of "these two are apart": `distinct` asks it to stay
    /// idempotent, `merge` to refuse a fold a person forbade, and the proximity listing to label a
    /// candidate. Compared by origin, like every verdict.
    pub(crate) async fn kept_apart<C: ConnectionTrait>(
        &self,
        conn: &C,
        a: i32,
        b: i32,
    ) -> anyhow::Result<bool> {
        let pair = pair_key(a, b);
        Ok(model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Distinct.as_str()))
            .all(conn)
            .await?
            .iter()
            .any(|verdict| {
                matches!(
                    (verdict.winner_origin, verdict.loser_origin),
                    (Some(a), Some(b)) if pair_key(a, b) == pair
                )
            }))
    }

    /// Record that two entities are deliberately separate. Standing like a veto: matched by the
    /// pair's origins, independent of generation; it has no effect on the graph by itself.
    pub(crate) async fn distinct(
        &self,
        generation: i32,
        entity_a: i32,
        entity_b: i32,
        note: Option<&str>,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(entity_a != entity_b, "an entity cannot be distinct from itself");
        let rows = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::Id.is_in([entity_a, entity_b]))
            .all(&self.conn)
            .await?;
        for id in [entity_a, entity_b] {
            let Some(row) = rows.iter().find(|row| row.id == id) else {
                anyhow::bail!("no knowledge entity {id}");
            };
            anyhow::ensure!(
                row.merged_into_id.is_none(),
                "knowledge entity {id} is a tombstone"
            );
        }
        let a = rows.iter().find(|row| row.id == entity_a).unwrap();
        let b = rows.iter().find(|row| row.id == entity_b).unwrap();
        let (Some(origin_a), Some(origin_b)) = (a.origin_observation_id, b.origin_observation_id)
        else {
            anyhow::bail!("an entity without an origin observation cannot be named by a verdict");
        };
        let pair = pair_key(origin_a, origin_b);

        if self.kept_apart(&self.conn, origin_a, origin_b).await? {
            return Ok(false);
        }
        // A pair that is already queued for a person's fold must not also be marked apart.
        let pending_merge = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Merge.as_str()))
            .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
            .all(&self.conn)
            .await?;
        anyhow::ensure!(
            !pending_merge.iter().any(|verdict| {
                matches!(
                    (verdict.winner_origin, verdict.loser_origin),
                    (Some(a), Some(b)) if pair_key(a, b) == pair
                )
            }),
            "the pair already has a standing merge verdict"
        );

        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(generation),
            cause: Set(Cause::Repair),
            kind: Set(Kind::Distinct),
            winner_origin: Set(Some(origin_a)),
            loser_origin: Set(Some(origin_b)),
            winner_label: Set(Some(a.name.clone())),
            loser_label: Set(Some(b.name.clone())),
            note: Set(note.map(str::to_string)),
            ..Default::default()
        })
        .exec(&self.conn)
        .await?;
        Ok(true)
    }
}

/// The unordered pair two origins name, as a key.
pub(crate) fn pair_key(a: i32, b: i32) -> (i32, i32) {
    (a.min(b), a.max(b))
}
