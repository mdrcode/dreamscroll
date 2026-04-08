use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, webhook};

/// Webhook POST route for Cloud Tasks ingest payloads.
///
/// Expected body is raw JSON for `IngestTask`, e.g.:
/// `{ "capture_id": 123 }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task): Json<webhook::schema::IngestTask>,
) -> Result<impl IntoResponse, api::ApiError> {
    webhook::logic::illuminate::exec(
        &state.service_api,
        &state.illuminator,
        webhook::schema::IlluminationTask {
            capture_id: task.capture_id,
        },
    )
    .await?;

    webhook::logic::search_index::exec(
        &state.service_api,
        state.stg.as_ref(),
        &state.embedder,
        &state.vector_store,
        webhook::schema::SearchIndexTask {
            capture_id: task.capture_id,
        },
    )
    .await?;

    tracing::info!(
        capture_id = task.capture_id,
        "Ingest completed: illumination + search indexing"
    );

    Ok(StatusCode::NO_CONTENT)
}
