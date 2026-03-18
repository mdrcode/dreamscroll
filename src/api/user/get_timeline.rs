use sea_orm::prelude::*;
use sea_orm::{QueryOrder, QuerySelect};

use crate::{api::*, auth, database::DbHandle, model};

pub async fn get_timeline_captures(
    db: &DbHandle,
    user_context: &auth::Context,
    limit: u64,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    if limit == 0 {
        return Ok(vec![]);
    }

    let capture_ids = model::capture::Entity::find()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .limit(limit)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|capture| capture.id)
        .collect::<Vec<i32>>();

    if capture_ids.is_empty() {
        return Ok(vec![]);
    }

    let captures = super::get_captures(db, user_context, capture_ids).await?;

    Ok(captures)
}
