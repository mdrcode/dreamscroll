use anyhow::anyhow;

use crate::api;

use super::*;

#[derive(Clone)]
pub enum WebhookAuth {
    None,
    PubSubOidc(gcloud::PubSubOidcVerifier),
}

impl WebhookAuth {
    pub async fn verify(&self, headers: &axum::http::HeaderMap) -> Result<(), api::ApiError> {
        match self {
            WebhookAuth::None => Ok(()),
            WebhookAuth::PubSubOidc(verifier) => {
                let token = gcloud::extract_bearer_token(headers)?;
                verifier.verify_bearer_token(token).await.map_err(|err| {
                    tracing::warn!("Pub/Sub OIDC verification failed: {}", err);
                    api::ApiError::unauthorized(anyhow!(
                        "OIDC verification failed for Pub/Sub webhook: {}",
                        err
                    ))
                })
            }
        }
    }
}
