use sea_orm::prelude::*;

use crate::{api::*, database::DbHandle, model};

// Note that this is not user-specific, so it doesn't take a context and
// returns all captures matching the ids, regardless of user.
pub async fn get_captures(
    db: &DbHandle,
    ids: Option<Vec<i32>>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    let mut loader = model::capture::Entity::load();

    if let Some(ids) = &ids {
        loader = loader.filter(model::capture::Column::Id.is_in(ids.clone()));
    }

    let loader = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with((model::illumination::Entity, model::xquery::Entity))
        .with((model::illumination::Entity, model::knode::Entity))
        .with((model::illumination::Entity, model::social_media::Entity))
        .all(&db.conn)
        .await?;

    Ok(loader)
}
