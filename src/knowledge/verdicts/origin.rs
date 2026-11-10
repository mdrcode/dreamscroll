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

    /// Follow tombstones to the live entity a row ends at. Bounded: a cycle is not expressible in
    /// the schema, but the reader must not spin if one somehow appears.
    pub(crate) async fn canonical(&self, id: i32) -> anyhow::Result<Option<i32>> {
        let mut current = id;
        for _ in 0..8 {
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
        Ok(None)
    }
}
