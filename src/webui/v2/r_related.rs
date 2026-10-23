use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::WebState;

pub async fn previews(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<api::CapturePreviewInfo>>, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let previews = state
        .user_api
        .search_similar_previews(&context_user, id, Some(12))
        .await?;

    Ok(Json(previews))
}
