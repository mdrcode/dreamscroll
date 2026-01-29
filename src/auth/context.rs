use super::*;

/// Security context for business logic which is agnostic to the authentication
/// mechanism (session-based, JWT, etc.). The ultimate (aspirational) goal is
/// that this type encapsulates both the identity and authorization information
/// about the user. So if an administrator wants to perform some action on
/// behalf of another user (i.e. authorized impersonation), the context would
/// reflect both the admin's authority and the target user's identity.
///
/// Context instances can ONLY be created via:
/// - `From<DreamscrollAuthUser>` for user-based authentication
/// - `from_service_credentials()` for service-to-service authentication
///
/// Direct construction of variants is prevented by private inner types.
#[derive(Debug, Clone)]
#[allow(private_interfaces)]
pub enum Context {
    User(UserInner),
    Service(ServiceInner),
}

/// Private wrapper for user context data.
/// Prevents direct construction of `Context::User(...)`.
#[derive(Debug, Clone)]
struct UserInner(DreamscrollAuthUser);

/// Private wrapper for service context data.
/// Prevents direct construction of `Context::Service(...)`.
#[derive(Debug, Clone)]
struct ServiceInner(String);

impl Context {
    /// Validates the service token and returns a `Context::Service` if successful.
    ///
    /// # Returns
    ///
    /// A `Context::Service` with the verified service name, or an error if validation fails.
    pub fn from_service_credentials(
        jwt_config: &JwtConfig,
        token: String,
    ) -> Result<Self, AuthError> {
        let service_name = jwt_config
            .decode_service_token(&token)
            .map(|claims| claims.service_name)
            .inspect_err(|e| tracing::warn!("Service token verification failed: {}", e))?;
        Ok(Context::Service(ServiceInner(service_name)))
    }

    /// Returns true if this is a user context.
    pub fn is_user(&self) -> bool {
        matches!(self, Context::User(_))
    }

    /// Returns true if this is a service context.
    pub fn is_service(&self) -> bool {
        matches!(self, Context::Service(_))
    }

    pub fn is_admin(&self) -> bool {
        match self {
            Context::User(user_inner) => user_inner.0.is_admin(),
            Context::Service(_) => true,
        }
    }

    pub fn user_id(&self) -> i32 {
        match self {
            Context::User(user_inner) => user_inner.0.user_id(),
            Context::Service(_) => 0, // System user ID
        }
    }

    /// Returns the service name if this is a service context.
    pub fn service_name(&self) -> Option<&str> {
        match self {
            Context::User(_) => None,
            Context::Service(svc_ctx) => Some(&svc_ctx.0),
        }
    }
}

/// Converts a `DreamscrollAuthUser` into a `Context::User`.
impl From<DreamscrollAuthUser> for Context {
    fn from(user: DreamscrollAuthUser) -> Self {
        Context::User(UserInner(user))
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
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let token = config
            .create_service_token("illuminator")
            .expect("should create service token");
        let context =
            Context::from_service_credentials(&config, token).expect("should create context");

        assert_eq!(context.user_id(), 0);
    }

    #[test]
    fn test_service_context_is_admin() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let token = config
            .create_service_token("scheduler")
            .expect("should create service token");
        let context =
            Context::from_service_credentials(&config, token).expect("should create context");

        assert!(context.is_admin());
    }

    #[test]
    fn test_service_context_is_debuggable() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let token = config
            .create_service_token("notifier")
            .expect("should create service token");
        let context =
            Context::from_service_credentials(&config, token).expect("should create context");

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

        let ctx = Context::from_service_credentials(&config, token)
            .expect("should create service context");

        assert_eq!(ctx.service_name(), Some("illuminator"));
        assert!(ctx.is_service());
        assert!(!ctx.is_user());
    }

    #[test]
    fn test_service_context_from_invalid_credentials_fails() {
        let config = JwtConfig::from_secret(b"test-secret-32-bytes-minimum!!!");
        let token = "invalid.garbage.token".to_string();

        let result = Context::from_service_credentials(&config, token);
        assert!(result.is_err());
    }

    #[test]
    fn test_service_context_from_wrong_secret_fails() {
        let config1 = JwtConfig::from_secret(b"secret-one-at-least-32-bytes!!!");
        let config2 = JwtConfig::from_secret(b"secret-two-at-least-32-bytes!!!");

        let token = config1
            .create_service_token("scheduler")
            .expect("should create service token");

        // Try to verify with a different secret
        let result = Context::from_service_credentials(&config2, token);
        assert!(result.is_err());
    }
}
