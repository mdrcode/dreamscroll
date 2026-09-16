use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks spark inference payloads.
///
/// Expected body is a serialized `TaskEnvelope<SparkTask>`, e.g.:
/// `{ "user_id": 1, "task_id": "u1-spark-...", "task": { "capture_ids": [123, 456] } }`
///
/// REVISIT: status tracking is currently minimal — `attempts` is hardcoded to
/// `0`, and a failed `exec` writes `Error` directly (no `ErrorFinal` escalation
/// or retry policy yet). See `_project/plans/sse-task-status.md`.
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

    state
        .task_master
        .update_status(&envelope, task::StatusCode::InProgress, 0)
        .await
        .map_err(api::ApiError::internal)?;

    let result = logic::spark::exec(&state.service_api, state.firestarter.as_ref(), task).await;

    let status = if result.is_ok() {
        task::StatusCode::Completed
    } else {
        task::StatusCode::Error
    };
    state
        .task_master
        .update_status(&envelope, status, 0)
        .await
        .map_err(api::ApiError::internal)?;

    result?;

    Ok(StatusCode::NO_CONTENT)
}
