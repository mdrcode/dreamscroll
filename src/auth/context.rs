use super::*;

/// Security context for business logic which is agnostic to the authentication
/// mechanism (session-based, JWT, etc.). The ultimate (aspirational) goal is
/// that this type encapsulates both the identity and authorization information
/// about the user. So if an administrator wants to perform some action on
/// behalf of another user, the context would reflect both the admin's
/// authority and the target user's identity.
///
/// Currently, Context instances can be created via either:
/// - `DreamscrollAuthUser` for user-based authentication, using From<>
/// - `service::ServiceCredentials` for service-to-service authentication, using from_service_credentials()
#[derive(Debug, Clone)]
pub enum Context {
    User(DreamscrollAuthUser),
    Service(String),
}

impl Context {
    /// Validates the service token and returns a `Context::Service` if successful.
    ///
    /// # Returns
    ///
    /// A `Context::Service` with the verified service name, or an error if validation fails.
    pub fn from_service_credentials(
        jwt_config: &JwtConfig,
        creds: &service::ServiceCredentials,
    ) -> Result<Self, AuthError> {
        let service_name = service::verify_token(jwt_config, creds)?;
        Ok(Context::Service(service_name))
    }

    pub fn user_id(&self) -> i32 {
        match self {
            Context::User(user) => user.user_id(),
            Context::Service(_) => 0, // System user ID
        }
    }

    pub fn is_admin(&self) -> bool {
        match self {
            Context::User(user) => user.is_admin(),
            Context::Service(_) => true,
        }
    }
}

/// Converts a `DreamscrollAuthUser` into a `Context::User`.
impl From<DreamscrollAuthUser> for Context {
    fn from(user: DreamscrollAuthUser) -> Self {
        Context::User(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::JwtUserClaims;

    // ========================================================================
    // User Context Tests
    // ========================================================================

    #[test]
    fn test_user_context_from_session_auth() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let context = Context::from(user);

        assert_eq!(context.user_id(), 42);
        assert!(!context.is_admin());
    }

    #[test]
    fn test_user_context_from_jwt_auth() {
        let claims = JwtUserClaims {
            sub: 123,
            exp: 9999999999,
            iat: 1000000000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        let context = Context::from(user);

        assert_eq!(context.user_id(), 123);
    }

    #[test]
    fn test_user_context_is_cloneable() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let context = Context::from(user);
        let cloned = context.clone();

        assert_eq!(context.user_id(), cloned.user_id());
    }

    #[test]
    fn test_user_context_is_debuggable() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let context = Context::from(user);

        let debug_str = format!("{:?}", context);
        assert!(debug_str.contains("User"));
    }

    // ========================================================================
    // Service Context Tests
    // ========================================================================

    #[test]
    fn test_service_context_has_system_user_id() {
        let context = Context::Service("illuminator".to_string());

        assert_eq!(context.user_id(), 0);
    }

    #[test]
    fn test_service_context_is_admin() {
        let context = Context::Service("scheduler".to_string());

        assert!(context.is_admin());
    }

    #[test]
    fn test_service_context_is_debuggable() {
        let context = Context::Service("notifier".to_string());

        let debug_str = format!("{:?}", context);
        assert!(debug_str.contains("Service"));
        assert!(debug_str.contains("notifier"));
    }

    #[test]
    fn test_service_context_from_valid_credentials() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let token = config
            .create_service_token("illuminator")
            .expect("should create service token");
        let creds = service::ServiceCredentials { token };

        let ctx = Context::from_service_credentials(&config, &creds)
            .expect("should create service context");

        match ctx {
            Context::Service(name) => assert_eq!(name, "illuminator"),
            Context::User(_) => panic!("Expected Service context, got User"),
        }
    }

    #[test]
    fn test_service_context_from_invalid_credentials_fails() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let creds = service::ServiceCredentials {
            token: "invalid.garbage.token".to_string(),
        };

        let result = Context::from_service_credentials(&config, &creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_service_context_from_wrong_secret_fails() {
        let config1 = JwtConfig::from_secret(b"secret-one-at-least-32-bytes!!!");
        let config2 = JwtConfig::from_secret(b"secret-two-at-least-32-bytes!!!");

        let token = config1
            .create_service_token("scheduler")
            .expect("should create service token");
        let creds = service::ServiceCredentials { token };

        // Try to verify with a different secret
        let result = Context::from_service_credentials(&config2, &creds);
        assert!(result.is_err());
    }
}
