use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks spark inference payloads.
///
/// Expected body is a serialized `TaskEnvelope<SparkTask>`, e.g.:
/// `{ "user_id": 1, "envelope_id": "u1-spark-spark5", "task": { "capture_ids": [123, 456] } }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(envelope): Json<task::TaskEnvelope<logic::spark::SparkTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task = envelope.task.clone();

    if task.capture_ids.is_empty() {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&envelope)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    let result = logic::spark::exec(&state.service_api, state.firestarter.as_ref(), task).await;

    let outcome = state
        .task_master
        .finish_attempt(&envelope, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
