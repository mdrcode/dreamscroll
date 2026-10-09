use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks search indexing payloads.
///
/// Expected body is a serialized `TaskRun<SearchIndexTask>`, e.g.:
/// `{ "user_id": 1, "logical_id": "u1-search_index-capture123", "run_id": "2d3954ac-c392-4aa0-933a-1663e597c444", "run_number": 1, "task": { "capture_id": 123 } }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task_run): Json<task::TaskRun<logic::search_index::SearchIndexTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task = task_run.task.clone();

    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&task_run)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    let result = logic::search_index::exec(&state.logic, task).await;

    let outcome = state
        .task_master
        .finish_attempt(&task_run, attempt, &result, None)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
