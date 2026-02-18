use anyhow::anyhow;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::{api, auth};

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

pub fn decode_payload<P>(encoded: &str) -> Result<P, api::ApiError>
where
    P: serde::de::DeserializeOwned,
{
    let bytes = STANDARD.decode(encoded).map_err(|err| {
        api::ApiError::bad_request(anyhow!("Invalid base64 in Pub/Sub message data: {err}"))
    })?;

    serde_json::from_slice::<P>(&bytes).map_err(|err| {
        api::ApiError::bad_request(anyhow!(
            "Invalid JSON payload in Pub/Sub message data: {err}"
        ))
    })
}

#[derive(Clone)]
pub enum InternalWebhookAuth {
    None,
    BearerToken(String),
    PubSubOidc(std::sync::Arc<auth::PubSubOidcVerifier>),
}

pub async fn validate_internal_webhook_auth(
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

pub fn validate_bearer_token(
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<(), api::ApiError> {
    let token = extract_bearer_token(headers)?;

    if token != expected_token {
        return Err(api::ApiError::unauthorized(anyhow!("Invalid bearer token")));
    }

    Ok(())
}

pub fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, api::ApiError> {
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
