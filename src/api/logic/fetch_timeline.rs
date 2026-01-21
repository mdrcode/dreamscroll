use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder};

use crate::{api, auth, common::AppError, database::DbHandle, entity::*};

// TODO obviously this should take a user_id or equivalent at some point
pub async fn fetch_timeline(
    user_context: auth::Context,
    db: &DbHandle,
) -> anyhow::Result<Vec<api::CaptureInfo>, AppError> {
    let captures = capture::Entity::load()
        .filter(capture::Column::UserId.eq(user_context.user_id()))
        .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
        .with(media::Entity)
        .all(&db.conn)
        .await?;

    let illuminations = captures
        .load_many(
            illumination::Entity::find()
                //.filter(illumination::Column::Provider.eq("gemini"))
                .order_by(illumination::Column::Id, sea_orm::Order::Desc),
            &db.conn,
        )
        .await?;

    let capture_infos = captures
        .into_iter()
        .zip(illuminations.into_iter())
        .map(|(c, ill)| {
            let mut mx = c;
            mx.illuminations =
                HasMany::Loaded(ill.into_iter().map(illumination::ModelEx::from).collect());
            api::CaptureInfo::from(mx)
        })
        .collect();

    Ok(capture_infos)
}
