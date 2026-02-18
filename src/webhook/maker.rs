use std::sync::Arc;

use anyhow::anyhow;
use axum::{Router, routing::post};

use crate::{api, illumination, storage};

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

pub struct WebhookState {
    // This processor is intentionally backed by ServiceApiClient and therefore
    // does not require user auth/JWT context. Internal background services are
    // treated as elevated trusted components because they have DB access.
    pub service_api: api::ServiceApiClient,
    pub auth: WebhookAuth,
    pub illuminator: Box<dyn illumination::Illuminator>,
}

pub fn make_router(
    service_api: api::ServiceApiClient,
    stg: Box<dyn storage::StorageProvider>,
    auth: WebhookAuth,
) -> Router {
    let state = Arc::new(WebhookState {
        service_api: service_api,
        auth,
        illuminator: illumination::make_illuminator("geministructured", stg.clone()),
    });

    Router::new()
        .route("/illumination/push", post(r_wh_illuminate::post))
        .with_state(state)
}
