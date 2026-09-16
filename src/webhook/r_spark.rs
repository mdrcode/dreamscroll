use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks spark inference payloads.
///
/// Expected body is a serialized `TaskEnvelope<SparkTask>`, e.g.:
/// `{ "user_id": 1, "task_id": "u1-spark-...", "task": { "capture_ids": [123, 456] } }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(envelope): Json<task::TaskEnvelope<logic::spark::SparkTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let Some(task) = envelope.task.clone() else {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "TaskEnvelope missing payload"
        )));
    };

    if task.capture_ids.is_empty() {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    let attempt = state
        .task_master
        .begin_attempt(&envelope)
        .await
        .map_err(api::ApiError::internal)?;

    let result = logic::spark::exec(&state.service_api, state.firestarter.as_ref(), task).await;

    let outcome = state
        .task_master
        .finish_attempt(&envelope, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    // See webhook::http_status_for_outcome for why exhausted errors still ack.
    Ok(webhook::http_status_for_outcome(outcome))
}
