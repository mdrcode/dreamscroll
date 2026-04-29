use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is raw JSON for `IlluminationTask`, e.g.:
/// `{ "capture_id": 123 }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task): Json<webhook::schema::IlluminationTask>,
) -> Result<impl IntoResponse, api::ApiError> {
    webhook::logic::illuminate::exec(&state.service_api, state.illuminator.as_ref(), task).await?;

    Ok(StatusCode::NO_CONTENT)
}
