use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks spark inference payloads.
///
/// Expected body is a serialized `TaskRun<SparkTask>`, e.g.:
/// `{ "user_id": 1, "logical_id": "u1-spark-spark5", "run_id": "9a6f177e-64ed-4ced-8da6-b6a2ec43687d", "run_number": 1, "task": { "capture_ids": [123, 456] } }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task_run): Json<task::TaskRun<logic::spark::SparkTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task = task_run.task.clone();

    if task.capture_ids.is_empty() {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&task_run)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    let result = logic::spark::exec(&state.logic, task).await;

    let outcome = state
        .task_master
        .finish_attempt(&task_run, attempt, &result, None)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
