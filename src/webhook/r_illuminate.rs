use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is a serialized `TaskEnvelope<IlluminationTask>`, e.g.:
/// `{ "user_id": 1, "task_id": "u1-illuminate-...", "task": { "capture_id": 123 } }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(envelope): Json<task::TaskEnvelope<logic::illuminate::IlluminationTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let Some(task) = envelope.task.clone() else {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "TaskEnvelope missing payload"
        )));
    };

    let attempt = state
        .task_master
        .begin_attempt(&envelope)
        .await
        .map_err(api::ApiError::internal)?;

    let result = logic::illuminate::exec(&state.service_api, state.illuminator.as_ref(), task).await;

    let outcome = state
        .task_master
        .finish_attempt(&envelope, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    // See webhook::http_status_for_outcome for why exhausted errors still ack.
    Ok(webhook::http_status_for_outcome(outcome))
}
