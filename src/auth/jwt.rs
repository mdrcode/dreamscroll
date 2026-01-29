//! JWT (JSON Web Token) authentication support for Dreamscroll.
//!
//! This module provides:
//! - `JwtConfig`: Configuration for JWT encoding/decoding (keys, expiration)
//! - `JwtUserClaims`: Claims describing the authenticated user in the token
//! - `JwtServiceClaims`: Claims for service-to-service authentication tokens
//! - `FromRequestParts` implementation for extracting and validating JWTs in
//!   Axum and converting to `DreamscrollAuthUser`
//! - `JwtAxumLayer`: Support for adding JWT config to Axum request extensions
//! - `JwtAxumMiddleware`: Middleware to inject JWT config into requests
//!
//! # Usage
//!
//! Add `DreamscrollAuthUser` as an extractor parameter in your route handlers:
//!
//! ```ignore
//! async fn protected_route(user: DreamscrollAuthUser) -> impl IntoResponse {
//!     let context = auth::Context::from(&user);
//!     // ... use context for business logic
//! }
//! ```
//!
//! The JWT is expected in the `Authorization` header as `Bearer <token>`.

use std::sync::Arc;

use axum::{RequestPartsExt, extract::FromRequestParts, http::request::Parts};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use super::{AuthError, DreamscrollAuthUser};

const DEFAULT_JWT_USER_EXPIRATION_SECS: u64 = 24 * 60 * 60;
const DEFAULT_JWT_SERVICE_EXPIRATION_SECS: u64 = 7 * 24 * 60 * 60;

/// Configuration for JWT token generation and validation.
///
/// This struct holds the cryptographic keys and settings needed for JWT operations.
/// Initialize once at application startup and share via `Arc<JwtConfig>`.
///
/// # Example
///
/// ```ignore
/// let config = JwtConfig::from_secret(b"your-secret-key");
/// let token = config.create_user_token(user_id)?;
/// ```
#[derive(Clone)]
pub struct JwtConfig {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    /// User token expiration duration in seconds
    user_expiration_secs: u64,
    /// Service token expiration duration in seconds
    service_expiration_secs: u64,
    /// Leeway for token expiration validation (seconds)
    leeway: u64,
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
            user_expiration_secs: DEFAULT_JWT_USER_EXPIRATION_SECS,
            service_expiration_secs: DEFAULT_JWT_SERVICE_EXPIRATION_SECS,
            leeway: 0,
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

    /// Sets a custom expiration duration for user tokens.
    ///
    /// This is a builder method that returns self for chaining.
    ///
    /// # Arguments
    ///
    /// * `secs` - The expiration duration in seconds.
    pub fn with_user_expiration_secs(mut self, secs: u64) -> Self {
        self.user_expiration_secs = secs;
        self
    }

    /// Sets a custom expiration duration for service tokens.
    ///
    /// This is a builder method that returns self for chaining.
    ///
    /// # Arguments
    ///
    /// * `secs` - The expiration duration in seconds.
    pub fn with_service_expiration_secs(mut self, secs: u64) -> Self {
        self.service_expiration_secs = secs;
        self
    }

    /// Sets a custom leeway for token expiration validation.
    ///
    /// Leeway allows tokens to be accepted for a short period after their
    /// expiration time to account for clock skew between servers.
    ///
    /// # Arguments
    ///
    /// * `secs` - The leeway duration in seconds.
    pub fn with_leeway(mut self, secs: u64) -> Self {
        self.leeway = secs;
        self
    }

    /// Returns the configured user token expiration duration in seconds.
    pub fn user_expiration_secs(&self) -> u64 {
        self.user_expiration_secs
    }

    /// Returns the configured service token expiration duration in seconds.
    pub fn service_expiration_secs(&self) -> u64 {
        self.service_expiration_secs
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
    pub fn create_user_token(&self, user: DreamscrollAuthUser) -> Result<String, AuthError> {
        let now = jsonwebtoken::get_current_timestamp();
        let claims = JwtUserClaims {
            sub: user.user_id(),
            exp: now + self.user_expiration_secs,
            iat: now,
        };

        encode(&Header::default(), &claims, &self.encoding_key).map_err(AuthError::from)
    }

    /// Validates and decodes a JWT token.
    ///
    /// # Returns
    ///
    /// The decoded claims, or an error if validation fails.
    pub fn decode_user_token(&self, token: &str) -> Result<JwtUserClaims, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // We only require exp claim (which is validated automatically)
        validation.required_spec_claims.clear();
        validation.required_spec_claims.insert("exp".to_string());
        validation.leeway = self.leeway;

        let token_data = decode::<JwtUserClaims>(token, &self.decoding_key, &validation)?;
        Ok(token_data.claims)
    }

    /// Creates a JWT token for a service account.
    ///
    /// Service tokens identify backend processes (e.g., "illuminator", "scheduler")
    /// and grant admin-level access for system operations.
    ///
    /// # Arguments
    ///
    /// * `service_name` - Identifier for the service
    ///
    /// # Returns
    ///
    /// The encoded JWT string, or an error if encoding fails.
    pub fn create_service_token(&self, service_name: &str) -> Result<String, AuthError> {
        let now = jsonwebtoken::get_current_timestamp();
        let claims = JwtServiceClaims {
            service_name: service_name.to_string(),
            exp: now + self.service_expiration_secs,
            iat: now,
        };

        encode(&Header::default(), &claims, &self.encoding_key).map_err(AuthError::from)
    }

    /// Validates and decodes a service JWT token.
    ///
    /// # Returns
    ///
    /// The decoded service claims, or an error if validation fails.
    pub fn decode_service_token(&self, token: &str) -> Result<JwtServiceClaims, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.required_spec_claims.clear();
        validation.required_spec_claims.insert("exp".to_string());
        validation.leeway = self.leeway;

        let token_data = decode::<JwtServiceClaims>(token, &self.decoding_key, &validation)?;
        Ok(token_data.claims)
    }
}

impl std::fmt::Debug for JwtConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtConfig")
            .field("user_expiration_secs", &self.user_expiration_secs)
            .field("service_expiration_secs", &self.service_expiration_secs)
            .field("leeway", &self.leeway)
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
#[serde(deny_unknown_fields)]
pub struct JwtUserClaims {
    /// Subject: the user ID this token represents
    pub sub: i32,
    /// Expiration time as UTC timestamp (seconds since epoch)
    pub exp: u64,
    /// Issued-at time as UTC timestamp
    pub iat: u64,
}

impl JwtUserClaims {
    /// Returns the user ID from the claims.
    pub fn user_id(&self) -> i32 {
        self.sub
    }
}
/// Claims embedded in a service JWT token.
///
/// Service tokens identify backend processes rather than users.
/// They are used for system-to-system authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JwtServiceClaims {
    /// The name of the service (e.g., "illuminator", "scheduler")
    pub service_name: String,
    /// Expiration time as UTC timestamp (seconds since epoch)
    pub exp: u64,
    /// Issued-at time as UTC timestamp
    pub iat: u64,
}

impl JwtServiceClaims {
    /// Returns the service name from the claims.
    pub fn service_name(&self) -> &str {
        &self.service_name
    }
}

/// Axum extractor that validates JWT tokens from the Authorization header.
///
/// Use this in route handlers to require authentication. The extractor will:
/// 1. Extract the `Authorization: Bearer <token>` header
/// 2. Validate and decode the JWT
/// 3. Return the authenticated user information as `DreamscrollAuthUser`
///
/// If validation fails, the request is rejected with an appropriate HTTP status.
///
/// # Example
///
/// ```ignore
/// async fn my_protected_handler(user: DreamscrollAuthUser) -> impl IntoResponse {
///     format!("Hello, user {}", user.user_id())
/// }
/// ```
///
/// Axum extractor implementation for DreamscrollAuthUser.
///
/// Requires `Arc<JwtConfig>` to be present in the request extensions (added via layer).
impl<S> FromRequestParts<S> for DreamscrollAuthUser
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract the JWT config from request extensions
        let jwt_config = parts
            .extensions
            .get::<Arc<JwtConfig>>()
            .ok_or_else(|| {
                tracing::error!(
                    "JwtConfig not found in request extensions. Did you add the JWT layer?"
                );
                AuthError::InvalidToken
            })?
            .clone();

        // Extract the Authorization: Bearer <token> header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::MissingOrInvalidHeader)?;

        // Decode and validate the token
        let claims = jwt_config.decode_user_token(bearer.token())?;

        Ok(DreamscrollAuthUser::from(claims))
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
///     .layer(JwtAxumLayer::new(config));
/// ```
#[derive(Clone)]
pub struct JwtAxumLayer {
    config: Arc<JwtConfig>,
}

impl JwtAxumLayer {
    pub fn new(config: Arc<JwtConfig>) -> Self {
        Self { config }
    }
}

impl<S> tower::Layer<S> for JwtAxumLayer {
    type Service = JwtAxumMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        JwtAxumMiddleware {
            inner,
            config: self.config.clone(),
        }
    }
}

/// Middleware that injects JWT config into request extensions.
#[derive(Clone)]
pub struct JwtAxumMiddleware<S> {
    inner: S,
    config: Arc<JwtConfig>,
}

impl<S, B> tower::Service<axum::http::Request<B>> for JwtAxumMiddleware<S>
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

    // ========================================================================
    // User Token Tests
    // ========================================================================

    #[test]
    fn test_user_token_create_and_decode() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");
        let user = DreamscrollAuthUser::new_test_session(42);

        let token = config
            .create_user_token(user.clone())
            .expect("should create user token");
        assert!(!token.is_empty());

        let claims = config
            .decode_user_token(&token)
            .expect("should decode user token");
        assert_eq!(claims.user_id(), user.user_id());
    }

    #[test]
    fn test_user_token_invalid_rejected() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");

        let result = config.decode_user_token("invalid.token.here");
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[test]
    fn test_user_token_wrong_secret_rejected() {
        let config1 = JwtConfig::from_secret(b"secret-one-at-least-32-bytes!!!");
        let config2 = JwtConfig::from_secret(b"secret-two-at-least-32-bytes!!!");

        let user = DreamscrollAuthUser::new_test_session(42);
        let token = config1
            .create_user_token(user)
            .expect("should create user token");

        let result = config2.decode_user_token(&token);
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[test]
    fn test_user_token_contains_correct_claims() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");
        let user = DreamscrollAuthUser::new_test_session(99);

        let token = config
            .create_user_token(user)
            .expect("should create user token");
        let claims = config
            .decode_user_token(&token)
            .expect("should decode user token");

        assert_eq!(claims.sub, 99);
        assert!(claims.exp > claims.iat);
    }

    // ========================================================================
    // Service Token Tests
    // ========================================================================

    #[test]
    fn test_service_token_create_and_decode() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");

        let token = config
            .create_service_token("illuminator")
            .expect("should create service token");
        assert!(!token.is_empty());

        let claims = config
            .decode_service_token(&token)
            .expect("should decode service token");
        assert_eq!(claims.service_name(), "illuminator");
    }

    #[test]
    fn test_service_token_invalid_rejected() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");

        let result = config.decode_service_token("invalid.token.here");
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[test]
    fn test_service_token_wrong_secret_rejected() {
        let config1 = JwtConfig::from_secret(b"secret-one-at-least-32-bytes!!!");
        let config2 = JwtConfig::from_secret(b"secret-two-at-least-32-bytes!!!");

        let token = config1
            .create_service_token("scheduler")
            .expect("should create service token");

        let result = config2.decode_service_token(&token);
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[test]
    fn test_service_token_contains_correct_claims() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");

        let token = config
            .create_service_token("notifier")
            .expect("should create service token");
        let claims = config
            .decode_service_token(&token)
            .expect("should decode service token");

        assert_eq!(claims.service_name, "notifier");
        assert!(claims.exp > claims.iat);
    }

    // ========================================================================
    // Token Type Isolation Tests
    // These verify that user and service tokens cannot be confused.
    // ========================================================================

    #[test]
    fn test_user_token_cannot_be_decoded_as_service_token() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");
        let user = DreamscrollAuthUser::new_test_session(42);

        let user_token = config
            .create_user_token(user)
            .expect("should create user token");

        // Attempting to decode a user token as a service token should fail
        let result = config.decode_service_token(&user_token);
        assert!(
            result.is_err(),
            "user token should not decode as service token"
        );
    }

    #[test]
    fn test_service_token_cannot_be_decoded_as_user_token() {
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!");

        let service_token = config
            .create_service_token("illuminator")
            .expect("should create service token");

        // Attempting to decode a service token as a user token should fail
        let result = config.decode_user_token(&service_token);
        assert!(
            result.is_err(),
            "service token should not decode as user token"
        );
    }

    // ========================================================================
    // Token Expiration Tests
    // These verify that expired tokens are rejected.
    // ========================================================================

    #[test]
    fn test_user_token_expires() {
        // Create config with 1 second expiration
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!")
            .with_user_expiration_secs(1);
        let user = DreamscrollAuthUser::new_test_session(42);

        let token = config
            .create_user_token(user)
            .expect("should create user token");

        // Token should be valid immediately
        let result = config.decode_user_token(&token);
        assert!(result.is_ok(), "token should be valid immediately");

        // Wait for token to expire
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Token should now be expired
        let result = config.decode_user_token(&token);
        assert!(result.is_err(), "token should be expired after waiting");
    }

    #[test]
    fn test_service_token_expires() {
        // Create config with 1 second service token expiration
        let config = JwtConfig::from_secret(b"test-secret-key-at-least-32-bytes!")
            .with_service_expiration_secs(1);

        let token = config
            .create_service_token("illuminator")
            .expect("should create service token");

        // Token should be valid immediately
        let result = config.decode_service_token(&token);
        assert!(result.is_ok(), "service token should be valid immediately");

        // Wait for token to expire
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Token should now be expired
        let result = config.decode_service_token(&token);
        assert!(
            result.is_err(),
            "service token should be expired after waiting"
        );
    }
}
