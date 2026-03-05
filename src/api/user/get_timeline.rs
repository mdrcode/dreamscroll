use sea_orm::prelude::*;
use sea_orm::{QueryOrder, QuerySelect};

use crate::{api::*, auth, database::DbHandle, model};

pub async fn get_timeline(
    db: &DbHandle,
    user_context: &auth::Context,
    limit: Option<u64>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    let num_captures = limit.unwrap_or(100);
    if num_captures == 0 {
        return Ok(vec![]);
    }

    let capture_ids = model::capture::Entity::find()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .limit(num_captures)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|capture| capture.id)
        .collect::<Vec<i32>>();

    if capture_ids.is_empty() {
        return Ok(vec![]);
    }

    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .filter(model::capture::Column::Id.is_in(capture_ids))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with((model::illumination::Entity, model::xquery::Entity))
        .with((model::illumination::Entity, model::knode::Entity))
        .with((model::illumination::Entity, model::social_media::Entity))
        .all(&db.conn)
        .await?;

    Ok(captures)
}
