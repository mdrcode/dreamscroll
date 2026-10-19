use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{api, auth};

use super::RestState;

fn accepted_task_run_response(run_id: String) -> (axum::http::StatusCode, Json<String>) {
    (axum::http::StatusCode::ACCEPTED, Json(run_id))
}

#[derive(Debug, Deserialize)]
pub struct CaptureTaskRequest {
    pub capture_id: i32,
}

#[derive(Debug, Deserialize)]
pub struct IlluminateTaskRequest {
    pub capture_id: i32,
    #[serde(default)]
    pub prompt_version: crate::illumination::IlluminationVersion,
    pub thinking_level: Option<crate::llms::gemini::ThinkingLevel>,
    #[serde(default = "default_persist_projection")]
    pub persist_projection: bool,
}

fn default_persist_projection() -> bool {
    true
}

/// POST /api/queues/illuminate
pub async fn post_illuminate(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<IlluminateTaskRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let run_id = state
        .user_api
        .enqueue_illumination(
            &user.into(),
            request.capture_id,
            request.prompt_version,
            request.thinking_level,
            request.persist_projection,
        )
        .await?;
    Ok(accepted_task_run_response(run_id))
}

/// POST /api/queues/search_index
pub async fn post_search_index(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<CaptureTaskRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let run_id = state
        .user_api
        .enqueue_search_index(&user.into(), request.capture_id)
        .await?;
    Ok(accepted_task_run_response(run_id))
}

/// GET /api/tasks/{run_id}
pub async fn get_run(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(run_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let status = state.user_api.get_task_run(&user.into(), &run_id).await?;
    Ok(Json(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illuminate_request_defaults_to_v2_and_accepts_explicit_versions() {
        let default: IlluminateTaskRequest =
            serde_json::from_value(serde_json::json!({"capture_id": 42})).unwrap();
        assert_eq!(
            default.prompt_version,
            crate::illumination::IlluminationVersion::V2
        );
        assert_eq!(default.thinking_level, None);

        for (version, expected) in [
            ("v1", crate::illumination::IlluminationVersion::V1),
            ("v2", crate::illumination::IlluminationVersion::V2),
        ] {
            let request: IlluminateTaskRequest = serde_json::from_value(serde_json::json!({
                "capture_id": 42,
                "prompt_version": version
            }))
            .unwrap();
            assert_eq!(request.prompt_version, expected);
        }
    }

    #[test]
    fn illuminate_request_accepts_typed_thinking_levels_and_rejects_unknown_levels() {
        for (value, expected) in [
            ("minimal", crate::llms::gemini::ThinkingLevel::Minimal),
            ("low", crate::llms::gemini::ThinkingLevel::Low),
            ("medium", crate::llms::gemini::ThinkingLevel::Medium),
            ("high", crate::llms::gemini::ThinkingLevel::High),
        ] {
            let request: IlluminateTaskRequest = serde_json::from_value(serde_json::json!({
                "capture_id": 42,
                "thinking_level": value
            }))
            .unwrap();
            assert_eq!(request.thinking_level, Some(expected));
        }
        assert!(
            serde_json::from_value::<IlluminateTaskRequest>(serde_json::json!({
                "capture_id": 42,
                "thinking_level": "ultra"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn accepted_task_run_response_returns_json_run_id() {
        let run_id = "b91a7c4f-7e8a-4bf8-9a76-c81e258ec113";
        let response = accepted_task_run_response(run_id.to_string()).into_response();

        assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(serde_json::from_slice::<String>(&body).unwrap(), run_id);
    }
}
