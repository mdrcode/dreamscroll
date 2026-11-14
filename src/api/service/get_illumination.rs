use sea_orm::prelude::*;

use crate::{api::*, database::DbHandle, model};

/// One illumination with its observations loaded — the shape anything deciding about a reading needs.
///
/// Not user-scoped, like [`super::get_captures`]: a worker holds an id and asks for the row. Whether a
/// caller may act on that row is the caller's question, and it is asked where the id came from.
pub async fn get_illumination(
    db: &DbHandle,
    illumination_id: i32,
) -> Result<model::illumination::ModelEx, ApiError> {
    model::illumination::Entity::load()
        .filter(model::illumination::Column::Id.eq(illumination_id))
        .with(model::user_observed_entity::Entity)
        .one(&db.conn)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(anyhow::anyhow!("illumination {illumination_id} not found"))
        })
}
