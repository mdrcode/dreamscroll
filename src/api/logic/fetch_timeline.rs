use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api, auth, database::DbHandle, model};

pub async fn fetch_timeline(
    db: &DbHandle,
    user_context: &auth::Context,
) -> Result<Vec<api::CaptureInfo>, api::ApiError> {
    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::x_query::Entity)
        .with(model::k_node::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    let capture_infos = captures.into_iter().map(api::CaptureInfo::from).collect();

    Ok(capture_infos)
}
