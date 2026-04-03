use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, webhook};

/// Webhook POST route for Cloud Tasks spark inference payloads.
///
/// Expected body is raw JSON for `SparkTask`, e.g.:
/// `{ "capture_ids": [123, 456] }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task): Json<webhook::schema::SparkTask>,
) -> Result<impl IntoResponse, api::ApiError> {
    if task.capture_ids.is_empty() {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    webhook::logic::spark::exec(&state.service_api, &state.firestarter, task).await?;

    Ok(StatusCode::NO_CONTENT)
}
