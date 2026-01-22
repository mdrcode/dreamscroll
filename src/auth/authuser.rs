use crate::model::user;

use super::jwt::JwtClaims;

/// The authentication method used to create this user session.
#[derive(Clone, Debug)]
pub enum AuthMethod {
    /// Session-based authentication (e.g., cookie-based web login)
    Session {
        /// Hash used by axum-login for session validation
        session_hash: String,
    },
    /// JWT token-based authentication (e.g., API access)
    Jwt {
        /// The validated JWT claims
        claims: JwtClaims,
    },
}

/// Represents an authenticated user in the Dreamscroll system. We opted for
/// the name `DreamscrollAuthUser` because `AuthUser` is already taken by
/// axum-login as a key trait name.
///
/// # Construction
///
/// This type can ONLY be created through authenticated flows:
/// - `verify_password()` - after successful password verification  
/// - `From<JwtClaims>` - after successful JWT token validation
/// - `from_db_model()` (auth-module only) - for session rehydration by axum-login
///
/// There is intentionally no public way to construct this from a raw database
/// entity to prevent authentication bypass. The constructor is restricted to
/// the `auth` module using `pub(super)` visibility.
#[derive(Clone)]
pub struct DreamscrollAuthUser {
    id: i32,
    username: String,
    is_admin: bool,
    method: AuthMethod,
}

impl DreamscrollAuthUser {
    pub fn user_id(&self) -> i32 {
        self.id
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn is_admin(&self) -> bool {
        self.is_admin
    }

    pub fn auth_method(&self) -> &AuthMethod {
        &self.method
    }

    pub fn jwt_claims(&self) -> Option<&JwtClaims> {
        match &self.method {
            AuthMethod::Jwt { claims } => Some(claims),
            AuthMethod::Session { .. } => None,
        }
    }

    /// Creates a DreamscrollAuthUser from a database model.
    ///
    /// This is module-private (auth module only) and should only be used by:
    /// - `verify_password()` after successful password verification
    /// - `WebAuthBackend::get_user()` for session rehydration
    ///
    /// # Security Note
    ///
    /// This assumes the caller has already performed authentication checks.
    /// For password auth, the caller must verify the password hash.
    /// For session rehydration, axum-login has already validated the session.
    pub(super) fn from_db_model(user_model: user::Model) -> Self {
        Self {
            id: user_model.id,
            username: user_model.username,
            is_admin: user_model.is_admin,
            method: AuthMethod::Session {
                session_hash: user_model.password_hash,
            },
        }
    }

    #[cfg(test)]
    pub fn new_test_session(id: i32) -> Self {
        Self {
            id,
            username: format!("testuser{}", id),
            is_admin: false,
            method: AuthMethod::Session {
                session_hash: format!("test-hash-{}", id),
            },
        }
    }

    #[cfg(test)]
    pub fn new_test_jwt(id: i32, claims: JwtClaims) -> Self {
        Self {
            id,
            username: format!("jwtuser{}", id),
            is_admin: false,
            method: AuthMethod::Jwt { claims },
        }
    }
}

impl std::fmt::Debug for DreamscrollAuthUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("DreamscrollAuthUser");
        debug.field("id", &self.id);
        debug.field("username", &self.username);

        match &self.method {
            AuthMethod::Session { session_hash } => {
                debug.field("auth_method", &"Session");
                // Only show a prefix of the hash for security/readability
                let hash_preview = if session_hash.len() > 4 {
                    format!("{}...", &session_hash[..4])
                } else {
                    session_hash.clone()
                };
                debug.field("session_hash_preview", &hash_preview);
            }
            AuthMethod::Jwt { claims } => {
                debug.field("auth_method", &"Jwt");
                debug.field("jwt_sub", &claims.sub);
                debug.field("jwt_exp", &claims.exp);
                debug.field("jwt_iat", &claims.iat);
            }
        }

        debug.finish()
    }
}

// Axum-login uses this trait to extract user identity from the session
impl axum_login::AuthUser for DreamscrollAuthUser {
    type Id = i32;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        match &self.method {
            AuthMethod::Session { session_hash } => session_hash.as_bytes(),
            AuthMethod::Jwt { .. } => {
                // JWT users shouldn't be seen in a session validation context.
                tracing::error!("session_auth_hash called on JWT-auth user {:?}", self);
                debug_assert!(false, "session_auth_hash called on JWT-auth user");
                b""
            }
        }
    }
}

impl From<JwtClaims> for DreamscrollAuthUser {
    fn from(claims: JwtClaims) -> Self {
        let id = claims.sub;
        DreamscrollAuthUser {
            id,
            username: format!("jwtuser{}", id),
            is_admin: false, // TODO currently no is_admin info in JWT claims
            method: AuthMethod::Jwt { claims },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_login::AuthUser;

    #[test]
    fn test_user_id_session() {
        let user = DreamscrollAuthUser::new_test_session(42);
        assert_eq!(user.user_id(), 42);
    }

    #[test]
    fn test_user_id_jwt() {
        let claims = JwtClaims {
            sub: 123,
            exp: 9999,
            iat: 1000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        assert_eq!(user.user_id(), 123);
    }

    #[test]
    fn test_auth_method_session() {
        let user = DreamscrollAuthUser::new_test_session(42);
        match user.auth_method() {
            AuthMethod::Session { session_hash } => {
                assert_eq!(session_hash, "test-hash-42");
            }
            AuthMethod::Jwt { .. } => panic!("Expected Session auth method"),
        }
    }

    #[test]
    fn test_auth_method_jwt() {
        let claims = JwtClaims {
            sub: 123,
            exp: 9999,
            iat: 1000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims.clone());
        match user.auth_method() {
            AuthMethod::Jwt {
                claims: user_claims,
            } => {
                assert_eq!(user_claims.sub, 123);
                assert_eq!(user_claims.exp, 9999);
            }
            AuthMethod::Session { .. } => panic!("Expected Jwt auth method"),
        }
    }

    #[test]
    fn test_jwt_claims_returns_some_for_jwt() {
        let claims = JwtClaims {
            sub: 123,
            exp: 9999,
            iat: 1000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        let retrieved_claims = user.jwt_claims();
        assert!(retrieved_claims.is_some());
        assert_eq!(retrieved_claims.unwrap().sub, 123);
    }

    #[test]
    fn test_jwt_claims_returns_none_for_session() {
        let user = DreamscrollAuthUser::new_test_session(42);
        assert!(user.jwt_claims().is_none());
    }

    #[test]
    fn test_from_jwt_claims() {
        let claims = JwtClaims {
            sub: 456,
            exp: 8888,
            iat: 1111,
        };
        let user = DreamscrollAuthUser::from(claims.clone());

        assert_eq!(user.user_id(), 456);
        assert_eq!(user.jwt_claims().unwrap().sub, 456);
        assert_eq!(user.jwt_claims().unwrap().exp, 8888);
        assert_eq!(user.jwt_claims().unwrap().iat, 1111);
    }

    #[test]
    fn test_session_auth_hash_for_session() {
        let user = DreamscrollAuthUser::new_test_session(42);
        assert_eq!(user.session_auth_hash(), b"test-hash-42");
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "session_auth_hash called on JWT-auth user")
    )]
    fn test_session_auth_hash_for_jwt() {
        // JWT users should return empty bytes when session_auth_hash is called
        // In debug builds, this will panic due to debug_assert!
        // In release builds, it returns b"" and logs an error
        let claims = JwtClaims {
            sub: 123,
            exp: 9999,
            iat: 1000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        assert_eq!(user.session_auth_hash(), b"");
    }

    #[test]
    fn test_debug_format_session() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let debug_str = format!("{:?}", user);

        // Should contain user ID
        assert!(debug_str.contains("id: 42"));
        // Should indicate session auth method
        assert!(debug_str.contains("auth_method"));
        assert!(debug_str.contains("Session"));
        // Should show hash preview
        assert!(debug_str.contains("session_hash_preview"));
        // Test hash is "test-hash-42" which is > 4 chars, so should show first 4 chars
        assert!(debug_str.contains("test..."));
    }

    #[test]
    fn test_debug_format_session_short_hash() {
        // Create a custom user with a short hash for this specific test
        let user = DreamscrollAuthUser {
            id: 42,
            username: "shorthashuser".to_string(),
            is_admin: false,
            method: AuthMethod::Session {
                session_hash: "abc".to_string(),
            },
        };
        let debug_str = format!("{:?}", user);

        // For short hashes, should show full hash without "..."
        assert!(debug_str.contains("abc"));
        assert!(!debug_str.contains("..."));
    }

    #[test]
    fn test_debug_format_jwt() {
        let claims = JwtClaims {
            sub: 123,
            exp: 9999,
            iat: 1000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        let debug_str = format!("{:?}", user);

        // Should contain user ID
        assert!(debug_str.contains("id: 123"));
        // Should indicate JWT auth method
        assert!(debug_str.contains("auth_method"));
        assert!(debug_str.contains("Jwt"));
        // Should show JWT claim fields
        assert!(debug_str.contains("jwt_sub: 123"));
        assert!(debug_str.contains("jwt_exp: 9999"));
    }

    #[test]
    fn test_clone_session() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let cloned = user.clone();

        assert_eq!(user.user_id(), cloned.user_id());
        assert_eq!(user.session_auth_hash(), cloned.session_auth_hash());
    }

    #[test]
    fn test_clone_jwt() {
        let claims = JwtClaims {
            sub: 123,
            exp: 9999,
            iat: 1000,
        };
        let user = DreamscrollAuthUser::new_test_jwt(123, claims);
        let cloned = user.clone();

        assert_eq!(user.user_id(), cloned.user_id());
        assert_eq!(
            user.jwt_claims().unwrap().sub,
            cloned.jwt_claims().unwrap().sub
        );
        assert_eq!(
            user.jwt_claims().unwrap().exp,
            cloned.jwt_claims().unwrap().exp
        );
    }

    #[test]
    fn test_axum_login_id_trait() {
        let user = DreamscrollAuthUser::new_test_session(999);
        assert_eq!(user.id(), 999);
    }
}
