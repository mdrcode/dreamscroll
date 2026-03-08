use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::api;

use super::{WebhookState, r_wh_illuminate::IlluminationTask};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is raw JSON for `IlluminationTask`, e.g.:
/// `{ "capture_id": 123 }`
pub async fn post(
    State(state): State<Arc<WebhookState>>,
    Json(task): Json<IlluminationTask>,
) -> Result<impl IntoResponse, api::ApiError> {
    super::r_wh_illuminate::execute(&state.service_api, &state.illuminator, task.capture_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
