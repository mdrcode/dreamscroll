use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};

use crate::{api, auth, database::DbHandle, model};

pub async fn get_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Option<Vec<i32>>,
) -> Result<Vec<model::capture::ModelEx>, api::ApiError> {
    let mut loader = model::capture::Entity::load();

    // For user contexts, restrict to their own captures
    // TODO admin override?
    if context.is_user() {
        loader = loader.filter(model::capture::Column::UserId.eq(context.user_id()));
    }

    if let Some(ids) = &ids {
        loader = loader.filter(model::capture::Column::Id.is_in(ids.clone()));
    }

    let loader = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::xquery::Entity)
        .with(model::knode::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    Ok(loader)
}

pub async fn get_captures_need_illum(
    db: &DbHandle,
    context: &auth::Context,
) -> Result<Vec<i32>, api::ApiError> {
    if !context.is_service() {
        return Err(api::ApiError::unauthorized(anyhow!(
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

pub async fn get_captures_need_search_idx(
    db: &DbHandle,
    user_context: &auth::Context,
) -> Result<Vec<i32>, api::ApiError> {
    let captures_without_index = model::capture::Entity::find()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .left_join(model::search_index::Entity)
        .filter(model::search_index::Column::Id.is_null())
        .column(model::capture::Column::Id)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect::<Vec<i32>>();

    Ok(captures_without_index)
}
