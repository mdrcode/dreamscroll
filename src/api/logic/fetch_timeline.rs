use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api, auth, database::DbHandle, model};

// TODO obviously this should take a user_id or equivalent at some point
pub async fn fetch_timeline(
    user_context: auth::Context,
    db: &DbHandle,
) -> Result<Vec<api::CaptureInfo>, api::ApiError> {
    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with((model::illumination::Entity, model::x_query::Entity))
        .with((model::illumination::Entity, model::k_node::Entity))
        .all(&db.conn)
        .await?;

    let capture_infos = captures.into_iter().map(api::CaptureInfo::from).collect();

    Ok(capture_infos)
}
