use std::sync::Arc;

use google_cloud_auth::credentials::idtoken::verifier;

#[derive(Clone)]
pub struct PubSubOidcVerifier {
    verifier: Arc<verifier::Verifier>,
}

impl PubSubOidcVerifier {
    /// Creates an OIDC verifier for Pub/Sub push authenticated webhooks.
    ///
    /// This is a thin adapter around the official Google Rust auth verifier:
    /// `google_cloud_auth::credentials::idtoken::verifier`.
    ///
    /// Validation performed by the underlying verifier includes signature checks
    /// using JWKS, issuer and audience validation, token expiration checks, and
    /// optional email + email_verified checks when email is configured.
    pub fn new(
        expected_audience: String,
        expected_service_account_email: Option<String>,
        jwks_url: Option<String>,
    ) -> Self {
        let mut builder = verifier::Builder::new([expected_audience]);

        if let Some(email) = expected_service_account_email {
            builder = builder.with_email(email);
        }
        if let Some(url) = jwks_url {
            builder = builder.with_jwks_url(url);
        }

        Self {
            verifier: Arc::new(builder.build()),
        }
    }

    pub async fn verify_bearer_token(&self, bearer_token: &str) -> anyhow::Result<()> {
        self.verifier.verify(bearer_token).await?;
        Ok(())
    }
}
