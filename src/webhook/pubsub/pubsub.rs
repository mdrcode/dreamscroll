use anyhow::anyhow;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::api::ApiError;

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

pub fn decode_message_data<P>(data: &str) -> Result<P, ApiError>
where
    P: serde::de::DeserializeOwned,
{
    let bytes = STANDARD.decode(data).map_err(|err| {
        ApiError::bad_request(anyhow!("Invalid base64 in Pub/Sub message data: {err}"))
    })?;

    serde_json::from_slice::<P>(&bytes).map_err(|err| {
        ApiError::bad_request(anyhow!(
            "JSON deserialization error in Pub/Sub message data: {err}"
        ))
    })
}
