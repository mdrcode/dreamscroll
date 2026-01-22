use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};

use crate::{api, database::DbHandle, model};

pub async fn fetch_captures_need_illumination(
    db: &DbHandle,
) -> anyhow::Result<Vec<i32>, api::AppError> {
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
