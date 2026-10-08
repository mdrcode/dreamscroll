use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, illumination, logic, task, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is a serialized `TaskEnvelope<IlluminationTask>`, e.g.:
/// `{ "user_id": 1, "envelope_id": "u1-illuminate-capture123", "task": { "capture_id": 123, "model_id": "gemini-3.8-flash", "prompt_version": "v1" } }`
///
/// This is the live worker route for capture-created illumination tasks. It is
/// also the entry point for future backfill and rerun flows.
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(envelope): Json<task::TaskEnvelope<logic::illuminate::IlluminationTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task = &envelope.task;

    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&envelope)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    let (result, result_ref) = match logic::illuminate::exec(&state.logic, task).await {
        Ok((user_id, result_ref)) => {
            let result = match task.prompt_version {
                illumination::IlluminationVersion::V1 => {
                    logic::Beacon::new(state.task_master.clone())
                        .new_illumination(user_id, task.capture_id)
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
        .finish_attempt(&envelope, attempt, &result, result_ref)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
