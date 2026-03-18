use std::collections::HashMap;

use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

// This is user-specific, so it takes a context and only returns captures belonging to that user.
pub async fn get_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Vec<i32>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    if ids.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    let loader = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::Id.is_in(ids.clone()));

    let captures = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with((model::illumination::Entity, model::xquery::Entity))
        .with((model::illumination::Entity, model::knode::Entity))
        .with((model::illumination::Entity, model::social_media::Entity))
        .all(&db.conn)
        .await?;

    let mut captures_by_id: HashMap<i32, model::capture::ModelEx> = captures
        .into_iter()
        .map(|capture| (capture.id, capture))
        .collect();

    let ordered = ids
        .into_iter()
        .filter_map(|capture_id| captures_by_id.remove(&capture_id))
        .collect();

    Ok(ordered)
}
