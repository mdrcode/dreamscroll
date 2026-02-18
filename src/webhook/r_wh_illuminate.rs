use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::api;

use super::{WebhookState, gcloud};

#[derive(Debug, Deserialize)]
pub struct IlluminationTaskPayload {
    pub capture_id: i32,
}

/// Internal push endpoint for illumination tasks.
///
/// This endpoint intentionally bypasses user credentials and runs via the
/// ServiceApi-backed illumination processor.
///
/// Effective URL composition:
/// - Route segment defined in this module: `/illumination/push`
/// - Mounted by `make_internal_router()` under caller-provided prefix
/// - Cloudrun binary mounts internal router at `/internal`
/// - Therefore production path is: `/internal/illumination/push`
///
/// For Cloud Run deployments, Pub/Sub push subscriptions should target:
/// `https://<cloud-run-service-host>/internal/illumination/push`
///
/// Authentication is enforced by `InternalWebhookAuth` mode selected at app
/// startup by runtime configuration.
#[tracing::instrument(skip(state, headers, body), fields(capture_id))]
pub async fn post(
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    Json(body): Json<gcloud::PubSubPushBody>,
) -> Result<impl IntoResponse, api::ApiError> {
    state.auth.verify(&headers).await?;

    let payload = gcloud::decode_payload::<IlluminationTaskPayload>(&body.message.data)?;

    tracing::Span::current().record("capture_id", payload.capture_id);

    if let Some(message_id) = &body.message.message_id {
        tracing::info!(message_id, "Processing Pub/Sub illumination push message");
    }
    if let Some(subscription) = &body.subscription {
        tracing::debug!(subscription, "Pub/Sub subscription source");
    }

    state
        .processor
        .process_capture_id(payload.capture_id)
        .await
        .map_err(|err| {
            tracing::error!(
                capture_id = payload.capture_id,
                error = ?err,
                "Failed processing Pub/Sub illumination task"
            );
            err
        })?;

    Ok(StatusCode::NO_CONTENT)
}
