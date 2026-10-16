use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is a serialized `TaskRun<IlluminationTask>`, e.g.:
/// `{ "logical_id": "u1-illuminate-capture123", "run_id": "b91a7c4f-7e8a-4bf8-9a76-c81e258ec113", "run_number": 1, "task": { "capture_id": 123, "entity_owner_id": 1, "model_id": "gemini-3.8-flash", "prompt_version": "v2" } }`
///
/// This is the worker route for capture-created illumination, backfill, and
/// rerun tasks. Backfill metadata is inherited by follow-up indexing tasks.
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task_run): Json<task::TaskRun<logic::illuminate::IlluminationTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task = &task_run.task;

    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&task_run)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    let result = match logic::illuminate::exec(&state.logic, task, &task_run.run_id).await {
        Ok(()) => logic::Beacon::new(state.task_master.clone())
            .new_illumination(task, task_run.backfill_id.as_deref())
            .await
            .map_err(api::ApiError::internal),
        Err(error) => Err(error),
    };

    let outcome = state
        .task_master
        .finish_attempt(&task_run, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
