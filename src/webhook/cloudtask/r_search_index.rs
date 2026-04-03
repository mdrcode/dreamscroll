use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, webhook};

/// Webhook POST route for Cloud Tasks search indexing payloads.
///
/// Expected body is raw JSON for `SearchIndexTask`, e.g.:
/// `{ "capture_id": 123 }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task): Json<webhook::schema::SearchIndexTask>,
) -> Result<impl IntoResponse, api::ApiError> {
    webhook::logic::search_index::exec(&state.service_api, &state.search_indexer, task).await?;

    Ok(StatusCode::NO_CONTENT)
}
