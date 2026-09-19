use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for Cloud Tasks illumination payloads.
///
/// Expected body is a serialized `TaskEnvelope<IlluminationTask>`, e.g.:
/// `{ "user_id": 1, "envelope_id": "u1-illuminate-capture123", "task": { "capture_id": 123 } }`
///
/// This is the app's core unit of work: `logic::illuminate::exec` runs both the
/// illumination and the search-indexing steps, since illumination has no real
/// purpose without search indexing.
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

    let result = logic::illuminate::exec(
        &state.service_api,
        state.illuminator.as_ref(),
        state.stg.as_ref(),
        &state.embedder,
        &state.vector_store,
        task,
    )
    .await;

    let outcome = state
        .task_master
        .finish_attempt(&envelope, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
