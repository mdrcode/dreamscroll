use super::*;

/// Security context for business logic which is agnostic to the authentication
/// mechanism (session-based, JWT, etc.).
///
/// Context instances can ONLY be created via `From<DreamscrollAuthUser>`
/// Direct construction of variants is prevented by private inner types.
#[derive(Debug, Clone)]
pub struct Context {
    user: DreamscrollAuthUser,
}

impl Context {
    pub fn is_admin(&self) -> bool {
        self.user.is_admin()
    }

    pub fn user_id(&self) -> i32 {
        self.user.user_id()
    }
}

/// Converts a `DreamscrollAuthUser` into a `Context::User`. This is the
/// only way to create a `Context`, ensuring that all necessary information is
/// properly encapsulated and a context is always associated with a valid user.
impl From<DreamscrollAuthUser> for Context {
    fn from(user: DreamscrollAuthUser) -> Self {
        Context { user }
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
}
