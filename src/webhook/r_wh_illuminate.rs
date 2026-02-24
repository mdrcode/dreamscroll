use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{api, illumination};

use super::*;

#[derive(Debug, Deserialize)]
pub struct IlluminationPayload {
    pub capture_id: i32,
}

/// Webhook POST endpoint for illumination tasks.
///
///
/// For Cloud Run deployments, Pub/Sub push subscriptions should target:
/// `https://<cloud-run-service-host>/webhook/illumination/push`
///
/// Authentication is enforced by `WebhookAuth` configured in `maker::make_router`.
pub async fn post(
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    Json(body): Json<PushBody>,
) -> Result<impl IntoResponse, api::ApiError> {
    state.auth.verify(&headers).await.map_err(|err| {
        tracing::error!(error = ?err, "Webhook authentication failed");
        api::ApiError::unauthorized(err)
    })?;

    let payload = decode_payload::<IlluminationPayload>(&body.message.data).map_err(|err| {
        tracing::error!(error = ?err, "Failed to decode Pub/Sub message payload");
        api::ApiError::bad_request(err)
    })?;

    tracing::Span::current().record("capture_id", payload.capture_id);

    if let Some(message_id) = &body.message.message_id {
        tracing::info!(message_id, "Processing Pub/Sub illumination push message");
    }
    if let Some(subscription) = &body.subscription {
        tracing::debug!(subscription, "Pub/Sub subscription source");
    }

    execute(&state.service_api, &state.illuminator, payload.capture_id)
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

pub async fn execute(
    service_api: &api::ServiceApiClient,
    illuminator: &Box<dyn illumination::Illuminator>,
    capture_id: i32,
) -> Result<(), api::ApiError> {
    let fetch = service_api.get_captures(Some(vec![capture_id])).await?;

    let Some(capture) = fetch.into_iter().next() else {
        tracing::warn!(capture_id, "Capture not found during illumination");
        return Ok(());
    };

    let illumination = match illuminator.illuminate(&capture).await {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(
                capture_id,
                error = ?err,
                "Illumination model call failed for capture"
            );
            return Err(api::ApiError::internal(err));
        }
    };

    service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(capture_id, "Illumination completed and inserted");
    Ok(())
}
