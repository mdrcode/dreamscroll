use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

/// Fetch the user's knowledge entities of the requested type.
pub async fn get_knowledge_entities_by_type(
    db: &DbHandle,
    context: &auth::Context,
    entity_type: &str,
) -> Result<Vec<model::knowledge_entity::Model>, ApiError> {
    Ok(model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::UserId.eq(context.user_id()))
        .filter(model::knowledge_entity::Column::EntityType.eq(entity_type))
        .order_by_asc(model::knowledge_entity::Column::Id)
        .all(&db.conn)
        .await?)
}
