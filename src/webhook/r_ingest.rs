use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, logic, webhook};

/// Webhook for overall "ingest" which does illumination and search indexing.
///
/// Expected body is raw JSON for `IngestTask`, e.g.:
/// `{ "capture_id": 123 }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task): Json<logic::ingest::IngestTask>,
) -> Result<impl IntoResponse, api::ApiError> {
    logic::ingest::exec(
        &state.service_api,
        state.illuminator.as_ref(),
        state.stg.as_ref(),
        &state.embedder,
        &state.vector_store,
        task,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}
