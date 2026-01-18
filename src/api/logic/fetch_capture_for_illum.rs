use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};

use crate::{common::AppError, database::DbHandle, entity::*};

pub async fn fetch_captures_need_illumination(db: &DbHandle) -> anyhow::Result<Vec<i32>, AppError> {
    let capture_ids = capture::Entity::find()
        .left_join(illumination::Entity)
        .filter(illumination::Column::Id.is_null())
        .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
        .select_only()
        .column(capture::Column::Id)
        .into_tuple::<i32>()
        .all(&db.conn)
        .await?;

    Ok(capture_ids)
}
