use std::sync::Arc;

use anyhow::Context;
use axum::{
    body::Body,
    http::{Request, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
};
use google_cloud_auth::credentials::idtoken::verifier::{Builder, Verifier};

use crate::config::Config;

pub fn oidc_from_config(cfg: &Config) -> anyhow::Result<Option<Arc<Verifier>>> {
    if cfg.task_backend == crate::config::TaskQueueBackend::Local {
        return Ok(None);
    }

    let service_account_email = cfg
        .task_oidc_service_account_email
        .as_deref()
        .context("OIDC service account required for Cloud Tasks")?;
    let audience = cfg
        .task_oidc_audience
        .as_deref()
        .context("OIDC audience required for Cloud Tasks")?;

    Ok(Some(make_verifier(service_account_email, audience)))
}

fn make_verifier(service_account_email: &str, audience: &str) -> Arc<Verifier> {
    // The Google verifier owns fetching and caching JWKS.
    Arc::new(
        Builder::new([audience])
            .with_email(service_account_email)
            .build(),
    )
}

pub async fn require_google_id_token(
    verifier: Arc<Verifier>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // Verify before deserializing or executing the task body.
    let Some(token) = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(extract_bearer_token)
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    match verifier.verify(token).await {
        Ok(_) => next.run(request).await,
        Err(error) => {
            // Do not expose token or verifier details to the caller.
            tracing::warn!(error = %error, "Webhook OIDC verification failed");
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

fn extract_bearer_token(header: &axum::http::HeaderValue) -> Option<&str> {
    header
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn extracts_bearer_token() {
        let header = HeaderValue::from_static("Bearer token-value");

        assert_eq!(extract_bearer_token(&header), Some("token-value"));
    }

    #[test]
    fn rejects_missing_or_malformed_bearer_tokens() {
        for value in ["", "Basic token-value", "Bearer ", "bearer token-value"] {
            let header = HeaderValue::from_static(value);
            assert_eq!(extract_bearer_token(&header), None, "header: {value:?}");
        }
    }

    #[test]
    fn rejects_non_utf8_authorization_headers() {
        let header = HeaderValue::from_bytes(b"Bearer \xff").expect("header value is valid");

        assert_eq!(extract_bearer_token(&header), None);
    }
}
