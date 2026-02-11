use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};

use crate::{api::*, auth, database::DbHandle, model};

pub async fn get_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Option<Vec<i32>>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    let mut loader =
        model::capture::Entity::load().filter(model::capture::Column::UserId.eq(context.user_id()));

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

pub async fn get_captures_need_illum(
    db: &DbHandle,
    context: &auth::Context,
) -> Result<Vec<i32>, ApiError> {
    if !context.is_service() {
        return Err(ApiError::unauthorized(anyhow!(
            "only service contexts can fetch captures needing illumination"
        )));
    }

    let capture_ids = model::capture::Entity::find()
        .left_join(model::illumination::Entity)
        .filter(model::illumination::Column::Id.is_null())
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .select_only()
        .column(model::capture::Column::Id)
        .into_tuple::<i32>()
        .all(&db.conn)
        .await?;

    Ok(capture_ids)
}
