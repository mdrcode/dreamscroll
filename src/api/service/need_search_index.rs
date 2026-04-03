use std::collections::HashSet;

use sea_orm::prelude::*;
use sea_orm::{EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::{api::*, database::DbHandle, model};

pub async fn get_captures_need_search_index(
    db: &DbHandle,
    limit: Option<u64>,
) -> Result<Vec<i32>, ApiError> {
    let ids = model::capture::Entity::find()
        .inner_join(model::illumination::Entity)
        .filter(model::capture::Column::ArchivedAt.is_null())
        .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
        .select_only()
        .column(model::capture::Column::Id)
        .into_tuple::<i32>()
        .all(&db.conn)
        .await?;

    // dedupe for the case where multiple illuminations exist for a capture
    // TODO we should handle this better
    let mut seen = HashSet::new();
    let mut capture_ids = Vec::with_capacity(ids.len());
    for id in ids {
        if seen.insert(id) {
            capture_ids.push(id);
        }
    }

    if let Some(limit) = limit {
        capture_ids.truncate(limit as usize);
    }

    tracing::warn!(
        "Currently this just returns recent captures without checking if they actually need search indexing. Candidate count may be inaccurate. Capture IDs: {:?}",
        capture_ids
    );

    Ok(capture_ids)
}
