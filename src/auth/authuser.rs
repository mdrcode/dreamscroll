use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

use crate::{database::DbHandle, entity::user};

use super::{autherror::*, jwt::JwtClaims};

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

/// Represents an authenticated user in the Dreamscroll system.
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
    method: AuthMethod,
}

impl DreamscrollAuthUser {
    pub fn user_id(&self) -> i32 {
        self.id
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
            method: AuthMethod::Session {
                session_hash: user_model.password_hash,
            },
        }
    }

    #[cfg(test)]
    pub fn new_test_session(id: i32) -> Self {
        Self {
            id,
            method: AuthMethod::Session {
                session_hash: format!("test-hash-{}", id),
            },
        }
    }

    #[cfg(test)]
    pub fn new_test_jwt(id: i32, claims: JwtClaims) -> Self {
        Self {
            id,
            method: AuthMethod::Jwt { claims },
        }
    }
}

impl std::fmt::Debug for DreamscrollAuthUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("DreamscrollAuthUser");
        debug.field("id", &self.id);

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
            method: AuthMethod::Jwt { claims },
        }
    }
}

pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = PasswordHash::generate(argon2, password.as_bytes(), &salt)?.to_string();
    Ok(password_hash)
}

pub enum Verification {
    Success(DreamscrollAuthUser),
    NoSuchUser,
    InvalidPassword,
}

pub async fn verify_password(db: &DbHandle, u: &str, p: &str) -> Result<Verification, AuthError> {
    let db_user = match user::Entity::find_by_username(u).one(&db.conn).await? {
        Some(user) => user,
        None => return Ok(Verification::NoSuchUser),
    };

    let parsed_hash = PasswordHash::new(&db_user.password_hash)?;

    if Argon2::default()
        .verify_password(p.as_bytes(), &parsed_hash)
        .is_ok()
    {
        Ok(Verification::Success(DreamscrollAuthUser::from_db_model(
            db_user,
        )))
    } else {
        Ok(Verification::InvalidPassword)
    }
}
