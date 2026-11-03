use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, logic, task, webhook};

/// Webhook POST route for the knowledge-link payloads: one illumination per task.
///
/// Expected body is a serialized `TaskRun<KnowledgeLinkTask>`, e.g.:
/// `{ "logical_id": "u1-knowledge_link-illumination7", "run_id": "…", "run_number": 1, "task": { "illumination_id": 7, "entity_owner_id": 1 } }`
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(task_run): Json<task::TaskRun<logic::knowledge_link::KnowledgeLinkTask>>,
) -> Result<impl IntoResponse, api::ApiError> {
    // `None` means the task already completed (at-least-once redelivery); ack it.
    let Some(attempt) = state
        .task_master
        .begin_attempt(&task_run)
        .await
        .map_err(api::ApiError::internal)?
    else {
        return Ok(axum::http::StatusCode::NO_CONTENT);
    };

    let result = logic::knowledge_link::exec(&state.logic, &task_run, attempt).await;

    let outcome = state
        .task_master
        .finish_attempt(&task_run, attempt, &result)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(webhook::http_status_for_task_run(outcome))
}
