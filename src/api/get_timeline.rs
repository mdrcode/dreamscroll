use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api, auth, database::DbHandle, model};

#[tracing::instrument(skip(db, user_context))]
pub async fn get_timeline(
    db: &DbHandle,
    user_context: &auth::Context,
) -> Result<Vec<model::capture::ModelEx>, api::ApiError> {
    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::xquery::Entity)
        .with(model::knode::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    Ok(captures)
}
