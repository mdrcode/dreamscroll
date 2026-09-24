use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use axum_extra::extract::Query;
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct SearchSimilarQuery {
    pub limit: Option<u64>,
}

/// GET /api/search/similar/{capture_id} - Find captures similar to a capture.
pub async fn get(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(capture_id): Path<i32>,
    Query(query): Query<SearchSimilarQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let captures = state
        .user_api
        .search_similar(&user.into(), capture_id, query.limit)
        .await?;

    Ok(Json(captures))
}
