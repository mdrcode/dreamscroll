use anyhow::anyhow;
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::{api::ApiError, pubsub};

#[derive(Clone)]
pub enum WebhookAuth {
    None,
    PubSubOidc(pubsub::gcloud::OidcVerifier),
}

impl WebhookAuth {
    pub async fn verify(&self, headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
        match self {
            WebhookAuth::None => Ok(()),
            WebhookAuth::PubSubOidc(verifier) => {
                let token = extract_bearer_token(headers)?;
                verifier.verify_bearer_token(token).await.map_err(|err| {
                    tracing::warn!("Pub/Sub OIDC verification failed: {}", err);
                    ApiError::unauthorized(anyhow!(
                        "OIDC verification failed for Pub/Sub webhook: {}",
                        err
                    ))
                })
            }
        }
    }
}

pub fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(ApiError::unauthorized(anyhow!(
            "Missing Authorization header"
        )));
    };

    let actual = value
        .to_str()
        .map_err(|_| ApiError::unauthorized(anyhow!("Invalid Authorization header encoding")))?;

    let Some(token) = actual.strip_prefix("Bearer ") else {
        return Err(ApiError::unauthorized(anyhow!(
            "Authorization must be Bearer token"
        )));
    };

    Ok(token)
}

#[derive(Debug, Deserialize)]
pub struct PushBody {
    pub message: PushMessage,
    pub subscription: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PushMessage {
    pub data: String,
    #[serde(rename = "messageId")]
    pub message_id: Option<String>,
}

pub fn decode_payload<P>(encoded: &str) -> Result<P, ApiError>
where
    P: serde::de::DeserializeOwned,
{
    let bytes = STANDARD.decode(encoded).map_err(|err| {
        ApiError::bad_request(anyhow!("Invalid base64 in Pub/Sub message data: {err}"))
    })?;

    serde_json::from_slice::<P>(&bytes).map_err(|err| {
        ApiError::bad_request(anyhow!(
            "Invalid JSON payload in Pub/Sub message data: {err}"
        ))
    })
}
