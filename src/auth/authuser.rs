use crate::model::user;

use super::autherror::AuthError;
use super::jwt::JwtUserClaims;

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
        claims: JwtUserClaims,
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
    storage_shard: String,
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

    pub fn storage_shard(&self) -> &str {
        &self.storage_shard
    }

    pub fn jwt_claims(&self) -> Option<&JwtUserClaims> {
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
            storage_shard: user_model.storage_shard,
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
            storage_shard: format!("testshard{}", id),
            method: AuthMethod::Session {
                session_hash: format!("test-hash-{}", id),
            },
        }
    }

    #[cfg(test)]
    pub fn new_test_jwt(id: i32, claims: JwtUserClaims) -> Self {
        Self {
            id,
            username: claims.username.clone(),
            is_admin: claims.is_admin,
            storage_shard: claims.storage_shard.clone(),
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
                debug.field("jwt_username", &claims.username);
                debug.field("jwt_is_admin", &claims.is_admin);
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
                // JWT users (via api token) should never be seen in a session validation context.
                tracing::error!(
                    "session_auth_hash called on JWT-auth user {:?}, should never happen",
                    self
                );
                panic!("session_auth_hash called on JWT-auth user, should never happen");
            }
        }
    }
}

impl TryFrom<JwtUserClaims> for DreamscrollAuthUser {
    type Error = AuthError;

    fn try_from(claims: JwtUserClaims) -> Result<Self, AuthError> {
        // sub is stored as a decimal string per RFC 7519; parse it back to i32.
        let id: i32 = claims.sub.parse().map_err(|_| AuthError::InvalidToken)?;
        let storage_shard = claims.storage_shard.clone();
        Ok(DreamscrollAuthUser {
            id,
            username: claims.username.clone(),
            is_admin: claims.is_admin,
            storage_shard,
            method: AuthMethod::Jwt { claims },
        })
    }
}

#[cfg(test)]
mod tests {
    use axum_login::AuthUser;

    use super::*;

    fn claims(sub: &str) -> JwtUserClaims {
        JwtUserClaims {
            sub: sub.to_string(),
            username: "alice".to_string(),
            is_admin: true,
            exp: 9_999_999_999,
            iat: 1_000_000_000,
            storage_shard: "shard-7".to_string(),
        }
    }

    #[test]
    fn from_jwt_claims_maps_identity_admin_and_shard() {
        let user = DreamscrollAuthUser::try_from(claims("456")).expect("numeric sub should decode");

        // `AuthUser::id` is the axum-login session key; the rest reach storage and authz.
        assert_eq!(user.id(), 456);
        assert_eq!(user.username(), "alice");
        assert!(user.is_admin());
        assert_eq!(user.storage_shard(), "shard-7");
        assert_eq!(user.jwt_claims().map(|c| c.exp), Some(9_999_999_999));
    }

    #[test]
    fn from_jwt_claims_rejects_a_sub_that_is_not_a_user_id() {
        let error = DreamscrollAuthUser::try_from(claims("not-a-number"))
            .expect_err("a non-numeric sub cannot name a user");

        assert!(matches!(error, AuthError::InvalidToken));
    }

    #[test]
    fn jwt_claims_are_absent_for_session_auth() {
        let user = DreamscrollAuthUser::new_test_session(42);
        assert!(user.jwt_claims().is_none());
    }

    #[test]
    fn session_auth_hash_is_the_stored_password_hash() {
        let user = DreamscrollAuthUser::new_test_session(42);
        assert_eq!(user.session_auth_hash(), b"test-hash-42");
    }

    #[test]
    #[should_panic(expected = "session_auth_hash called on JWT-auth user")]
    fn session_auth_hash_panics_for_jwt_auth() {
        // JWT users exist only for API requests; reaching session validation with one is a bug.
        let user = DreamscrollAuthUser::try_from(claims("123")).expect("numeric sub should decode");
        let _ = user.session_auth_hash();
    }

    #[test]
    fn debug_does_not_reveal_the_session_hash() {
        let user = DreamscrollAuthUser::new_test_session(42);
        let debug = format!("{user:?}");

        assert!(debug.contains("id: 42"));
        assert!(debug.contains("Session"));
        assert!(
            !debug.contains("test-hash-42"),
            "Debug leaked the session hash: {debug}"
        );
    }
}
