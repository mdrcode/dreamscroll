use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{api, auth, stats};

use super::{RestState, client::IlluminateTaskRequest, query::HoursQuery};

fn accepted_task_run_response(run_id: String) -> (axum::http::StatusCode, Json<String>) {
    (axum::http::StatusCode::ACCEPTED, Json(run_id))
}

#[derive(Debug, Deserialize)]
pub struct CaptureTaskRequest {
    pub capture_id: i32,
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
            request.thinking_level,
            request.persist_projection,
            request.web_search,
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

/// GET /api/admin/queue/stats?hours=24 - windowed queue timing and backlog (admin only).
pub async fn stats(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<HoursQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let windows = stats::parse_windows(query.hours.as_deref())?;
    let response = state
        .admin_api
        .queue_stats(&user.into(), &windows, state.task_backend)
        .await?;

    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct FailuresQuery {
    /// Window width in hours; defaults to 24.
    hours: Option<String>,
    /// Maximum rows; defaults to 20, clamped to 1..=200.
    limit: Option<i64>,
    /// Include runs currently in ErrorWillRetry.
    include_retrying: Option<bool>,
}

/// GET /api/admin/queue/failures?hours=24&limit=20 - recent failures (admin only).
pub async fn failures(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<FailuresQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let hours = stats::parse_window(query.hours.as_deref())?;
    let limit = query.limit.unwrap_or(20).clamp(1, 200);
    let response = state
        .admin_api
        .queue_failures(
            &user.into(),
            hours,
            limit,
            query.include_retrying.unwrap_or(false),
        )
        .await?;

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illuminate_request_defaults_to_provider_defaults_and_the_task_flags() {
        let default: IlluminateTaskRequest =
            serde_json::from_value(serde_json::json!({"capture_id": 42})).unwrap();
        assert_eq!(default.thinking_level, None);
        // Omitted flags defer to IlluminationTask's defaults rather than repeating them here.
        assert_eq!(default.persist_projection, None);
        assert_eq!(default.web_search, None);
    }

    #[test]
    fn illuminate_request_accepts_typed_thinking_levels_and_rejects_unknown_levels() {
        for (value, expected) in [
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
        for rejected in ["ultra", "minimal", "xhigh"] {
            // `minimal` and `xhigh` are real Gemini levels that our models reject, so they are
            // absent from ThinkingLevel; `ultra` is not a level at all.
            assert!(
                serde_json::from_value::<IlluminateTaskRequest>(serde_json::json!({
                    "capture_id": 42,
                    "thinking_level": rejected
                }))
                .is_err()
            );
        }
    }
}
