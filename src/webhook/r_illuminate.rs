use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

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
    let task = envelope.task.clone();

    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&envelope)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    // Use task identity plus attempt to trace each versioned inference result.
    let inference_run_id = format!(
        "{}-run{}-attempt{}",
        envelope.envelope_id, envelope.run, attempt
    );
    let result = match logic::illuminate::exec(&state.logic, task.clone(), inference_run_id).await {
        Ok(user_id) if task.prompt_version == logic::illuminate::IlluminationVersion::V1 => {
            logic::Beacon::new(state.task_master.clone())
                .new_illumination(user_id, task.capture_id)
                .await
                .map_err(api::ApiError::internal)
        }
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    };

    let outcome = state
        .task_master
        .finish_attempt(&envelope, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
