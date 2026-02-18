use std::sync::Arc;

use anyhow::anyhow;
use axum::{Router, routing::post};

use crate::{api, task};

use super::*;

#[derive(Clone)]
pub enum InternalWebhookAuth {
    None,
    PubSubOidc(std::sync::Arc<gcloud::PubSubOidcVerifier>),
}

impl InternalWebhookAuth {
    pub async fn validate(&self, headers: &axum::http::HeaderMap) -> Result<(), api::ApiError> {
        match self {
            InternalWebhookAuth::None => Ok(()),
            InternalWebhookAuth::PubSubOidc(verifier) => {
                let token = gcloud::extract_bearer_token(headers)?;
                verifier.verify_bearer_token(token).await.map_err(|err| {
                    api::ApiError::unauthorized(anyhow!(
                        "OIDC verification failed for Pub/Sub webhook: {}",
                        err
                    ))
                })
            }
        }
    }
}

pub struct InternalRestState {
    // This processor is intentionally backed by ServiceApiClient and therefore
    // does not require user auth/JWT context. Internal background services are
    // treated as elevated trusted components.
    pub processor: task::processor::CaptureIlluminationProcessor,
    pub webhook_auth: InternalWebhookAuth,
}

pub fn make_internal_router(
    processor: task::processor::CaptureIlluminationProcessor,
    webhook_auth: InternalWebhookAuth,
) -> Router {
    let state = Arc::new(InternalRestState {
        processor,
        webhook_auth,
    });

    Router::new()
        .route("/illumination/push", post(r_wh_illuminate::post))
        .with_state(state)
}
