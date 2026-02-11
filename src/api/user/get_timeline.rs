use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

pub async fn get_timeline(
    db: &DbHandle,
    user_context: &auth::Context,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
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
