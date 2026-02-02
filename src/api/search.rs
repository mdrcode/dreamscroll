use sea_orm::{EntityLoaderTrait, EntityTrait, QuerySelect, prelude::*};

use crate::{api, auth, database::DbHandle, model};

#[tracing::instrument(skip(db, user_context))]
pub async fn search_by_illuminations(
    db: &DbHandle,
    user_context: &auth::Context,
    query: &str,
) -> anyhow::Result<Vec<model::capture::ModelEx>, api::ApiError> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    let capture_ids_with_match = model::search_index::Entity::find()
        .filter(model::search_index::Column::UserId.eq(user_context.user_id()))
        .filter(model::search_index::Column::Content.contains(query))
        .column(model::search_index::Column::CaptureId)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|si| si.capture_id)
        .collect::<Vec<i32>>();

    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::Id.is_in(capture_ids_with_match))
        .order_by_id_desc()
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::xquery::Entity)
        .with(model::knode::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    Ok(captures)
}
