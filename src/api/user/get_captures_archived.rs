use sea_orm::prelude::*;
use sea_orm::{QueryOrder, QuerySelect};

use crate::{api::*, auth, database::DbHandle, model};

/// The user's archived captures, most recently archived first.
pub async fn get_captures_archived(
    db: &DbHandle,
    user_context: &auth::Context,
    limit: u64,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    if limit == 0 {
        return Ok(vec![]);
    }

    let capture_ids = model::capture::Entity::find()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .filter(model::capture::Column::ArchivedAt.is_not_null())
        .order_by(model::capture::Column::ArchivedAt, sea_orm::Order::Desc)
        // Entities archived in the same instant still list in a stable order.
        .order_by(model::capture::Column::Id, sea_orm::Order::Desc)
        .limit(limit)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|capture| capture.id)
        .collect::<Vec<i32>>();

    if capture_ids.is_empty() {
        return Ok(vec![]);
    }

    // Archived captures are the point of this query, so bypass the feed filter.
    super::get_captures_including_archived(db, user_context, capture_ids).await
}
