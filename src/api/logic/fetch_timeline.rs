use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder};

use crate::{api, auth, common::AppError, database::DbHandle, model};

// TODO obviously this should take a user_id or equivalent at some point
pub async fn fetch_timeline(
    user_context: auth::Context,
    db: &DbHandle,
) -> anyhow::Result<Vec<api::CaptureInfo>, AppError> {
    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .with(model::media::Entity)
        .all(&db.conn)
        .await?;

    let illuminations = captures
        .load_many(
            model::illumination::Entity::find()
                //.filter(model::illumination::Column::Provider.eq("gemini"))
                .order_by(model::illumination::Column::Id, sea_orm::Order::Desc),
            &db.conn,
        )
        .await?;

    let capture_infos = captures
        .into_iter()
        .zip(illuminations.into_iter())
        .map(|(c, ill)| {
            let mut mx = c;
            mx.illuminations = HasMany::Loaded(
                ill.into_iter()
                    .map(model::illumination::ModelEx::from)
                    .collect(),
            );
            api::CaptureInfo::from(mx)
        })
        .collect();

    Ok(capture_infos)
}
