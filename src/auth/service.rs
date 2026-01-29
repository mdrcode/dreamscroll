use super::*;

/// Credentials for service-to-service authentication.
///
/// Services authenticate using a pre-signed JWT token that contains
/// their service name. The token is validated cryptographically.
pub struct ServiceCredentials {
    /// The JWT token issued to this service
    pub token: String,
}

/// Verifies a service token and returns the authenticated service name.
///
/// # Arguments
///
/// * `jwt_config` - The JWT configuration used to validate the token
/// * `credentials` - The service credentials containing the token
///
/// # Returns
///
/// The verified service name from the token claims, or an error if validation fails.
pub fn verify_token(
    jwt_config: &JwtConfig,
    credentials: &ServiceCredentials,
) -> Result<String, AuthError> {
    jwt_config
        .decode_service_token(&credentials.token)
        .map(|claims| claims.service_name)
        .inspect_err(|e| tracing::warn!("Service token verification failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Service Token Verification Tests
    // ========================================================================

    #[test]
    fn test_verify_valid_service_token() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let token = config
            .create_service_token("illuminator")
            .expect("should create service token");
        let creds = ServiceCredentials { token };

        let name = verify_token(&config, &creds).expect("should verify service token");
        assert_eq!(name, "illuminator");
    }

    #[test]
    fn test_verify_invalid_service_token_fails() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let creds = ServiceCredentials {
            token: "invalid.garbage.token".to_string(),
        };

        let result = verify_token(&config, &creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_service_token_wrong_secret_fails() {
        let config1 = JwtConfig::from_secret(b"secret-one-at-least-32-bytes!!!");
        let config2 = JwtConfig::from_secret(b"secret-two-at-least-32-bytes!!!");

        let token = config1
            .create_service_token("scheduler")
            .expect("should create service token");
        let creds = ServiceCredentials { token };

        // Try to verify with a different secret
        let result = verify_token(&config2, &creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_service_token_preserves_service_name() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");

        // Test with various service names
        for service_name in ["illuminator", "scheduler", "notifier", "my-custom-service"] {
            let token = config
                .create_service_token(service_name)
                .expect("should create service token");
            let creds = ServiceCredentials { token };

            let verified_name = verify_token(&config, &creds).expect("should verify service token");
            assert_eq!(verified_name, service_name);
        }
    }

    #[test]
    fn test_verify_user_token_as_service_token_fails() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let user = DreamscrollAuthUser::new_test_session(42);

        // Create a user token
        let user_token = config
            .create_user_token(user)
            .expect("should create user token");
        let creds = ServiceCredentials { token: user_token };

        // Attempting to verify a user token as a service token should fail
        let result = verify_token(&config, &creds);
        assert!(
            result.is_err(),
            "user token should not verify as service token"
        );
    }
}
