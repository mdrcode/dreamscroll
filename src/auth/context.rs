use super::DreamscrollAuthUser;

/// Security context for business logic which is agnostic to the authentication
/// mechanism (session-based, JWT, etc.). The ultimate (aspirational) goal is
/// that this type encapsulates both the identity and authorization information
/// about the user. So if an administrator wants to perform some action on
/// behalf of another user, the context would reflect both the admin's
/// authority and the target user's identity.
///
/// Currently contains user identity information, with plans to extend with
/// permissions and roles as the authorization model evolves.
#[derive(Debug, Clone)]
pub struct Context {
    user: DreamscrollAuthUser,
    // TODO more to come
}

impl Context {
    pub fn user_id(&self) -> i32 {
        self.user.user_id()
    }
}

impl From<DreamscrollAuthUser> for Context {
    fn from(user: DreamscrollAuthUser) -> Self {
        Context { user }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::JwtClaims;

    #[test]
    fn test_context_from_session_auth_user() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let context = Context::from(user);

        assert_eq!(context.user_id(), 42);
    }

    #[test]
    fn test_context_from_jwt_auth_user() {
        let claims = JwtClaims {
            sub: 123,
            exp: 9999999999,
            iat: 1000000000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        let context = Context::from(user);

        assert_eq!(context.user_id(), 123);
    }

    #[test]
    fn test_context_is_cloneable() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let context = Context::from(user);
        let cloned = context.clone();

        assert_eq!(context.user_id(), cloned.user_id());
    }

    #[test]
    fn test_context_is_debuggable() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let context = Context::from(user);

        let debug_str = format!("{:?}", context);
        assert!(debug_str.contains("Context"));
    }
}
