use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QuerySelect};

use crate::{api, auth, database::DbHandle, model};

pub async fn get_illuminations(
    db: &DbHandle,
    context: &auth::Context,
    illumination_ids: Vec<i32>,
) -> Result<Vec<model::illumination::ModelEx>, api::ApiError> {
    let loader = model::illumination::Entity::load()
        .filter(model::illumination::Column::UserId.eq(context.user_id()))
        .filter(model::illumination::Column::Id.is_in(illumination_ids.clone()));

    let loader = loader
        .with(model::illumination_meta::Entity)
        .with(model::xquery::Entity)
        .with(model::knode::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    Ok(loader)
}

pub async fn get_illumination_ids_need_search(
    db: &DbHandle,
    user_context: &auth::Context,
) -> Result<Vec<i32>, api::ApiError> {
    let ids = model::illumination::Entity::find()
        .filter(model::illumination::Column::UserId.eq(user_context.user_id()))
        .left_join(model::search_index::Entity)
        .filter(model::search_index::Column::Id.is_null())
        .column(model::illumination::Column::Id)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect::<Vec<i32>>();

    Ok(ids)
}
