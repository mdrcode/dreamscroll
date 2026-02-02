use sea_orm::{ColumnTrait, QueryFilter};

use crate::{api, auth, database::DbHandle, model};

pub async fn fetch_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Option<Vec<i32>>,
) -> Result<Vec<model::capture::ModelEx>, api::ApiError> {
    let mut loader = model::capture::Entity::load();

    // For user contexts, restrict to their own captures
    // TODO admin override?
    if context.is_user() {
        loader = loader.filter(model::capture::Column::UserId.eq(context.user_id()));
    }

    if let Some(ids) = &ids {
        loader = loader.filter(model::capture::Column::Id.is_in(ids.clone()));
    }

    let loader = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::xquery::Entity)
        .with(model::knode::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    Ok(loader)
}
