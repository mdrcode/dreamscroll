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

/// Webhook POST endpoint for illumination tasks.
///
///
/// For Cloud Run deployments, Pub/Sub push subscriptions should target:
/// `https://<cloud-run-service-host>/webhook/illumination/push`
///
/// Authentication is enforced by `WebhookAuth` configured in `maker::make_router`.
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
