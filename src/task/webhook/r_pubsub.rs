use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::api;

use super::{InternalRestState, InternalWebhookAuth};

#[derive(Debug, Deserialize)]
pub struct PubSubPushBody {
    pub message: PubSubMessage,
    pub subscription: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PubSubMessage {
    pub data: String,
    #[serde(rename = "messageId")]
    pub message_id: Option<String>,
}

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
    State(state): State<Arc<InternalRestState>>,
    headers: HeaderMap,
    Json(body): Json<PubSubPushBody>,
) -> Result<impl IntoResponse, api::ApiError> {
    validate_internal_webhook_auth(&headers, &state.webhook_auth).await?;

    let payload = decode_payload(&body.message.data)?;

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

async fn validate_internal_webhook_auth(
    headers: &HeaderMap,
    auth: &InternalWebhookAuth,
) -> Result<(), api::ApiError> {
    match auth {
        InternalWebhookAuth::None => Ok(()),
        InternalWebhookAuth::BearerToken(expected_token) => {
            validate_bearer_token(headers, expected_token)
        }
        InternalWebhookAuth::PubSubOidc(verifier) => {
            let token = extract_bearer_token(headers)?;
            verifier.verify_bearer_token(token).await.map_err(|err| {
                api::ApiError::unauthorized(anyhow!(
                    "OIDC verification failed for Pub/Sub webhook: {}",
                    err
                ))
            })
        }
    }
}

fn validate_bearer_token(headers: &HeaderMap, expected_token: &str) -> Result<(), api::ApiError> {
    let token = extract_bearer_token(headers)?;

    if token != expected_token {
        return Err(api::ApiError::unauthorized(anyhow!("Invalid bearer token")));
    }

    Ok(())
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, api::ApiError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(api::ApiError::unauthorized(anyhow!(
            "Missing Authorization header"
        )));
    };

    let actual = value.to_str().map_err(|_| {
        api::ApiError::unauthorized(anyhow!("Invalid Authorization header encoding"))
    })?;

    let Some(token) = actual.strip_prefix("Bearer ") else {
        return Err(api::ApiError::unauthorized(anyhow!(
            "Authorization must be Bearer token"
        )));
    };

    Ok(token)
}

fn decode_payload(encoded: &str) -> Result<IlluminationTaskPayload, api::ApiError> {
    let bytes = STANDARD.decode(encoded).map_err(|err| {
        api::ApiError::bad_request(anyhow!("Invalid base64 in Pub/Sub message data: {err}"))
    })?;

    serde_json::from_slice::<IlluminationTaskPayload>(&bytes).map_err(|err| {
        api::ApiError::bad_request(anyhow!(
            "Invalid JSON payload in Pub/Sub message data: {err}"
        ))
    })
}
