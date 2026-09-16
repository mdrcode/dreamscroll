use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is a serialized `TaskEnvelope<IlluminationTask>`, e.g.:
/// `{ "user_id": 1, "task_id": "u1-illuminate-...", "task": { "capture_id": 123 } }`
///
/// REVISIT: status tracking is currently minimal — `attempts` is hardcoded to
/// `0`, and a failed `exec` writes `Error` directly (no `ErrorFinal` escalation
/// or retry policy yet). See `_project/plans/sse-task-status.md`.
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(envelope): Json<task::TaskEnvelope<logic::illuminate::IlluminationTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let Some(task) = envelope.task.clone() else {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "TaskEnvelope missing payload"
        )));
    };

    state
        .task_master
        .update_status(&envelope, task::StatusCode::InProgress, 0)
        .await
        .map_err(api::ApiError::internal)?;

    let result =
        logic::illuminate::exec(&state.service_api, state.illuminator.as_ref(), task).await;

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
