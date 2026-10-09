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
}

/// POST /api/queues/illuminate
pub async fn post_illuminate(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<IlluminateTaskRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let run_id = state
        .user_api
        .enqueue_illumination(&user.into(), request.capture_id, request.prompt_version)
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
    fn illuminate_request_defaults_older_payloads_to_v1_and_accepts_v2() {
        let legacy: IlluminateTaskRequest =
            serde_json::from_value(serde_json::json!({"capture_id": 42})).unwrap();
        assert_eq!(
            legacy.prompt_version,
            crate::illumination::IlluminationVersion::V1
        );

        let v2: IlluminateTaskRequest = serde_json::from_value(serde_json::json!({
            "capture_id": 42,
            "prompt_version": "v2"
        }))
        .unwrap();
        assert_eq!(
            v2.prompt_version,
            crate::illumination::IlluminationVersion::V2
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
