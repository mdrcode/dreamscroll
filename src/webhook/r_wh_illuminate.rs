use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use crate::{api, illumination};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationPayload {
    pub capture_id: i32,
}

/// Webhook POST route for illumination tasks.
///
/// For Cloud Run deployments, Pub/Sub push subscriptions should target:
/// `https://<cloud-run-service-host>/_wh/illumination/push`
///
/// There is NO AUTHENTICATION here, it should be enforced externally by GCloud ADC.
pub async fn post(
    State(state): State<Arc<WebhookState>>,
    Json(body): Json<PushBody>,
) -> Result<impl IntoResponse, api::ApiError> {
    let payload = decode_payload::<IlluminationPayload>(&body.message.data).map_err(|err| {
        tracing::error!(error = ?err, "Failed to decode Pub/Sub message payload");
        api::ApiError::bad_request(err)
    })?;

    execute(&state.service_api, &state.illuminator, payload.capture_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn execute(
    service_api: &api::ServiceApiClient,
    illuminator: &Box<dyn illumination::Illuminator>,
    capture_id: i32,
) -> Result<(), api::ApiError> {
    tracing::Span::current().record("capture_id", capture_id);

    let fetch = service_api.get_captures(Some(vec![capture_id])).await?;

    let Some(capture) = fetch.into_iter().next() else {
        tracing::warn!(capture_id, "Capture not found during illumination");
        return Ok(());
    };

    if !capture.illuminations.is_empty() {
        tracing::info!(
            capture_id,
            illumination_count = capture.illuminations.len(),
            "Idempotency guard: illumination already exists for capture; skipping"
        );
        return Ok(());
    }

    let illumination = illuminator.illuminate(&capture).await?;

    service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(capture_id, "Illumination completed and inserted");

    Ok(())
}
