use anyhow::anyhow;
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::api::ApiError;

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
