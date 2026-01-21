//! JWT (JSON Web Token) authentication support for the API.
//!
//! This module provides:
//! - `JwtConfig`: Configuration for JWT encoding/decoding (keys, expiration)
//! - `JwtClaims`: The claims embedded in a JWT token
//! - `JwtAuthUser`: An Axum extractor that validates JWT tokens and provides user context
//!
//! # Usage
//!
//! Add `JwtAuthUser` as an extractor parameter in your route handlers:
//!
//! ```ignore
//! async fn protected_route(user: JwtAuthUser) -> impl IntoResponse {
//!     let context = auth::Context::from(&user);
//!     // ... use context for business logic
//! }
//! ```
//!
//! The JWT is expected in the `Authorization` header as `Bearer <token>`.

use std::sync::Arc;

use axum::{
    RequestPartsExt,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

/// Duration in seconds for JWT token expiration (24 hours by default)
const DEFAULT_JWT_EXPIRATION_SECS: u64 = 24 * 60 * 60;

/// Configuration for JWT token generation and validation.
///
/// This struct holds the cryptographic keys and settings needed for JWT operations.
/// Initialize once at application startup and share via `Arc<JwtConfig>`.
///
/// # Example
///
/// ```ignore
/// let config = JwtConfig::from_secret(b"your-secret-key");
/// let token = config.create_token(user_id)?;
/// ```
#[derive(Clone)]
pub struct JwtConfig {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    /// Token expiration duration in seconds
    pub expiration_secs: u64,
}

impl JwtConfig {
    /// Creates a new JWT configuration from a secret key.
    ///
    /// Uses HS256 (HMAC-SHA256) symmetric encryption. The same secret is used
    /// for both signing and verification.
    ///
    /// # Arguments
    ///
    /// * `secret` - The secret key bytes. Should be at least 32 bytes for security.
    pub fn from_secret(secret: &[u8]) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
            expiration_secs: DEFAULT_JWT_EXPIRATION_SECS,
        }
    }

    /// Creates a new JWT configuration from an environment variable.
    ///
    /// Reads the secret from the specified environment variable.
    /// Panics if the variable is not set (fail-fast for configuration errors).
    ///
    /// # Arguments
    ///
    /// * `env_var` - The name of the environment variable containing the secret.
    pub fn from_env(env_var: &str) -> Self {
        let secret = std::env::var(env_var)
            .unwrap_or_else(|_| panic!("{env_var} environment variable must be set"));
        Self::from_secret(secret.as_bytes())
    }

    /// Sets a custom expiration duration for tokens.
    pub fn with_expiration_secs(mut self, secs: u64) -> Self {
        self.expiration_secs = secs;
        self
    }

    /// Creates a JWT token for the given user ID.
    ///
    /// The token includes:
    /// - `sub`: The user ID as the subject
    /// - `exp`: Expiration timestamp
    /// - `iat`: Issued-at timestamp
    ///
    /// # Returns
    ///
    /// The encoded JWT string, or an error if encoding fails.
    pub fn create_token(&self, user_id: i32) -> Result<String, JwtError> {
        let now = jsonwebtoken::get_current_timestamp();
        let claims = JwtClaims {
            sub: user_id,
            exp: now + self.expiration_secs,
            iat: now,
        };

        encode(&Header::default(), &claims, &self.encoding_key).map_err(JwtError::from)
    }

    /// Validates and decodes a JWT token.
    ///
    /// # Returns
    ///
    /// The decoded claims, or an error if validation fails.
    pub fn decode_token(&self, token: &str) -> Result<JwtClaims, JwtError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // We only require exp claim (which is validated automatically)
        validation.required_spec_claims.clear();
        validation.required_spec_claims.insert("exp".to_string());

        let token_data = decode::<JwtClaims>(token, &self.decoding_key, &validation)?;
        Ok(token_data.claims)
    }
}

impl std::fmt::Debug for JwtConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtConfig")
            .field("expiration_secs", &self.expiration_secs)
            .field("encoding_key", &"<redacted>")
            .field("decoding_key", &"<redacted>")
            .finish()
    }
}

/// Claims embedded in a JWT token.
///
/// These are the standard JWT claims plus any custom data we need.
/// The claims are cryptographically signed, so they can be trusted
/// after successful token validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject: the user ID this token represents
    pub sub: i32,
    /// Expiration time as UTC timestamp (seconds since epoch)
    pub exp: u64,
    /// Issued-at time as UTC timestamp
    pub iat: u64,
}

impl JwtClaims {
    /// Returns the user ID from the claims.
    pub fn user_id(&self) -> i32 {
        self.sub
    }
}

/// Axum extractor that validates JWT tokens from the Authorization header.
///
/// Use this in route handlers to require authentication. The extractor will:
/// 1. Extract the `Authorization: Bearer <token>` header
/// 2. Validate and decode the JWT
/// 3. Return the authenticated user information
///
/// If validation fails, the request is rejected with an appropriate HTTP status.
///
/// # Example
///
/// ```ignore
/// async fn my_protected_handler(user: JwtAuthUser) -> impl IntoResponse {
///     format!("Hello, user {}", user.user_id())
/// }
/// ```
#[derive(Debug, Clone)]
pub struct JwtAuthUser {
    claims: JwtClaims,
}

impl JwtAuthUser {
    /// Returns the authenticated user's ID.
    pub fn user_id(&self) -> i32 {
        self.claims.user_id()
    }

    /// Returns the full claims from the token.
    pub fn claims(&self) -> &JwtClaims {
        &self.claims
    }
}

/// Errors that can occur during JWT operations.
#[derive(Debug)]
pub enum JwtError {
    /// The Authorization header is missing or malformed
    MissingOrInvalidHeader,
    /// The token signature is invalid or the token is expired
    InvalidToken,
    /// An error occurred during token creation
    TokenCreation(String),
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JwtError::MissingOrInvalidHeader => {
                write!(f, "Missing or invalid Authorization header")
            }
            JwtError::InvalidToken => write!(f, "Invalid or expired token"),
            JwtError::TokenCreation(msg) => write!(f, "Token creation failed: {msg}"),
        }
    }
}

impl std::error::Error for JwtError {}

impl From<jsonwebtoken::errors::Error> for JwtError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        use jsonwebtoken::errors::ErrorKind;
        match err.kind() {
            ErrorKind::InvalidToken
            | ErrorKind::InvalidSignature
            | ErrorKind::ExpiredSignature
            | ErrorKind::ImmatureSignature
            | ErrorKind::Base64(_)
            | ErrorKind::Json(_)
            | ErrorKind::Utf8(_) => JwtError::InvalidToken,
            _ => JwtError::TokenCreation(err.to_string()),
        }
    }
}

impl IntoResponse for JwtError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            JwtError::MissingOrInvalidHeader => (StatusCode::UNAUTHORIZED, self.to_string()),
            JwtError::InvalidToken => (StatusCode::UNAUTHORIZED, self.to_string()),
            JwtError::TokenCreation(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}

/// Axum extractor implementation for JwtAuthUser.
///
/// Requires `Arc<JwtConfig>` to be present in the request extensions (added via layer).
impl<S> FromRequestParts<S> for JwtAuthUser
where
    S: Send + Sync,
{
    type Rejection = JwtError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract the JWT config from request extensions
        let jwt_config = parts
            .extensions
            .get::<Arc<JwtConfig>>()
            .ok_or_else(|| {
                tracing::error!(
                    "JwtConfig not found in request extensions. Did you add the JWT layer?"
                );
                JwtError::InvalidToken
            })?
            .clone();

        // Extract the Authorization: Bearer <token> header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| JwtError::MissingOrInvalidHeader)?;

        // Decode and validate the token
        let claims = jwt_config.decode_token(bearer.token())?;

        Ok(JwtAuthUser { claims })
    }
}

/// Axum layer that adds JWT configuration to request extensions.
///
/// Apply this layer to routes that need JWT authentication:
///
/// ```ignore
/// let config = Arc::new(JwtConfig::from_env("JWT_SECRET"));
/// let app = Router::new()
///     .route("/protected", get(handler))
///     .layer(JwtLayer::new(config));
/// ```
#[derive(Clone)]
pub struct JwtLayer {
    config: Arc<JwtConfig>,
}

impl JwtLayer {
    pub fn new(config: Arc<JwtConfig>) -> Self {
        Self { config }
    }
}

impl<S> tower::Layer<S> for JwtLayer {
    type Service = JwtMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        JwtMiddleware {
            inner,
            config: self.config.clone(),
        }
    }
}

/// Middleware that injects JWT config into request extensions.
#[derive(Clone)]
pub struct JwtMiddleware<S> {
    inner: S,
    config: Arc<JwtConfig>,
}

impl<S, B> tower::Service<axum::http::Request<B>> for JwtMiddleware<S>
where
    S: tower::Service<axum::http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: axum::http::Request<B>) -> Self::Future {
        req.extensions_mut().insert(self.config.clone());
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_decode_token() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");
        let user_id = 42;

        let token = config.create_token(user_id).expect("should create token");
        assert!(!token.is_empty());

        let claims = config.decode_token(&token).expect("should decode token");
        assert_eq!(claims.user_id(), user_id);
    }

    #[test]
    fn test_invalid_token_rejected() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");

        let result = config.decode_token("invalid.token.here");
        assert!(matches!(result, Err(JwtError::InvalidToken)));
    }

    #[test]
    fn test_wrong_secret_rejected() {
        let config1 = JwtConfig::from_secret(b"secret-one-at-least-32-bytes!!!");
        let config2 = JwtConfig::from_secret(b"secret-two-at-least-32-bytes!!!");

        let token = config1.create_token(42).expect("should create token");

        let result = config2.decode_token(&token);
        assert!(matches!(result, Err(JwtError::InvalidToken)));
    }
}
