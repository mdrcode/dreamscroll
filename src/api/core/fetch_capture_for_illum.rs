use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};

use crate::{api, auth, database::DbHandle, model};

pub async fn fetch_captures_need_illumination(
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
