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

    pub fn storage_shard(&self) -> &str {
        self.user.storage_shard()
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

    #[test]
    fn context_carries_identity_admin_and_shard_from_jwt_claims() {
        let claims = JwtUserClaims {
            sub: "7".to_string(),
            username: "admin".to_string(),
            is_admin: true,
            exp: 9_999_999_999,
            iat: 1_000_000_000,
            storage_shard: "shard-7".to_string(),
        };
        let context = Context::from(DreamscrollAuthUser::try_from(claims).expect("numeric sub"));

        assert_eq!(context.user_id(), 7);
        assert!(context.is_admin());
        assert_eq!(context.storage_shard(), "shard-7");
    }
}
