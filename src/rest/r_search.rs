use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
use axum_extra::extract::Query;
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<u64>,
}

/// GET /api/search - Search the authenticated user's captures.
pub async fn get(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<SearchQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    if query.q.trim().is_empty() {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "q must not be empty"
        )));
    }

    let captures = state
        .user_api
        .search(&user.into(), &query.q, query.limit)
        .await?;

    Ok(Json(captures))
}
