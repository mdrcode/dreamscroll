//! Turning a verdict's origin into the entity it names now.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::model;

use super::store::VerdictStore;

impl VerdictStore {
    /// The entity this origin names now, live or tombstoned.
    pub(crate) async fn entity_with_origin(
        &self,
        origin: i32,
    ) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
        Ok(model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::OriginObservationId.eq(origin))
            .one(&self.conn)
            .await?)
    }

    /// Follow tombstones to the live entity a row ends at.
    ///
    /// `Ok(None)` means the chain names no row at all: erosion, which the caller reports and drops.
    /// A chain longer than [`MAX_MERGE_HOPS`] is a **cycle** — the schema refuses only
    /// self-reference, so a hand-edited `A → B → A` is expressible — and that is an error, not
    /// erosion: reporting `None` there would drop a person's decision as if the entity had
    /// vanished.
    pub(crate) async fn canonical(&self, id: i32) -> anyhow::Result<Option<i32>> {
        let mut current = id;
        for _ in 0..MAX_MERGE_HOPS {
            let Some(row) = model::knowledge_entity::Entity::find_by_id(current)
                .one(&self.conn)
                .await?
            else {
                return Ok(None);
            };
            match row.merged_into_id {
                Some(next) => current = next,
                None => return Ok(Some(current)),
            }
        }
        anyhow::bail!(
            "merged_into_id chain from knowledge entity {id} exceeds {MAX_MERGE_HOPS} hops, so it \
             is a cycle the schema cannot refuse"
        )
    }
}

/// How far a tombstone chain may be followed before it is called a cycle. Merges chain — a replay
/// folds a pair a later merge already folded — but not this far; a real corpus stops at two or
/// three hops.
const MAX_MERGE_HOPS: usize = 8;
