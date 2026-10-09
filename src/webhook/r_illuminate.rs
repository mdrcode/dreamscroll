use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, illumination, logic, task, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is a serialized `TaskRun<IlluminationTask>`, e.g.:
/// `{ "user_id": 1, "logical_id": "u1-illuminate-capture123", "run_id": "b91a7c4f-7e8a-4bf8-9a76-c81e258ec113", "run_number": 1, "task": { "capture_id": 123, "model_id": "gemini-3.8-flash", "prompt_version": "v1" } }`
///
/// This is the live worker route for capture-created illumination tasks. It is
/// also the entry point for future backfill and rerun flows.
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

    let (result, result_ref) = match logic::illuminate::exec(&state.logic, task).await {
        Ok(result_ref) => {
            let result = match task.prompt_version {
                illumination::IlluminationVersion::V1 => {
                    logic::Beacon::new(state.task_master.clone())
                        .new_illumination(task_run.user_id, task.capture_id)
                        .await
                        .map_err(api::ApiError::internal)
                }
                illumination::IlluminationVersion::V2 => Ok(()),
            };
            let result_ref = result.is_ok().then_some(result_ref);
            (result, result_ref)
        }
        Err(error) => (Err(error), None),
    };

    let outcome = state
        .task_master
        .finish_attempt(&task_run, attempt, &result, result_ref)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
