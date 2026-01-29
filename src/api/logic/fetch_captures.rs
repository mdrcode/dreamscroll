use sea_orm::{ColumnTrait, EntityLoaderTrait, QueryFilter};

use crate::{api, auth, database::DbHandle, model};

pub async fn fetch_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Option<Vec<i32>>,
) -> Result<Vec<api::CaptureInfo>, api::ApiError> {
    let mut loader = model::capture::Entity::load();

    if let auth::Context::User(user) = &context {
        if !user.is_admin() {
            loader = loader.filter(model::capture::Column::UserId.eq(user.user_id()));
        }
    }

    if let Some(ids) = &ids {
        loader = loader.filter(model::capture::Column::Id.is_in(ids.clone()));
    }

    let loader = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::x_query::Entity)
        .with(model::k_node::Entity)
        .all(&db.conn)
        .await?;

    Ok(loader.into_iter().map(api::CaptureInfo::from).collect())
}
