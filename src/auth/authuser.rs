use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

use crate::{database::DbHandle, entity::user};

use super::{jwt::JwtClaims, webautherror::*};

#[derive(Clone)]
pub struct DreamscrollAuthUser {
    id: i32,
    session_hash: String,
    claims: Option<JwtClaims>,
}

impl DreamscrollAuthUser {
    pub fn user_id(&self) -> i32 {
        self.id
    }

    pub fn jwt_claims(&self) -> Option<&JwtClaims> {
        self.claims.as_ref()
    }

    #[cfg(test)]
    pub fn new_test(id: i32) -> Self {
        Self {
            id,
            session_hash: format!("test-hash-{}", id),
            claims: None,
        }
    }

    #[cfg(test)]
    pub fn new_test_with_jwt(id: i32, claims: JwtClaims) -> Self {
        Self {
            id,
            session_hash: String::new(),
            claims: Some(claims),
        }
    }
}

impl std::fmt::Debug for DreamscrollAuthUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamscrollAuthUser")
            .field("id", &self.id)
            .finish()
    }
}

// This trait tells axum-login how to identify your user
impl axum_login::AuthUser for DreamscrollAuthUser {
    type Id = i32;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        // SAFETY: This method should only be called by axum-login for
        // session-based authentication. JWT-authenticated users bypass
        // this trait entirely. If this panics, it indicates a logic error
        // in the authentication flow.
        assert!(
            !self.session_hash.is_empty(),
            "session_auth_hash called on JWT user (user_id: {})",
            self.id
        );
        self.session_hash.as_bytes()
    }
}

impl From<user::Model> for DreamscrollAuthUser {
    fn from(user_model: user::Model) -> Self {
        DreamscrollAuthUser {
            id: user_model.id,
            session_hash: user_model.password_hash,
            claims: None,
        }
    }
}

impl From<JwtClaims> for DreamscrollAuthUser {
    fn from(claims: JwtClaims) -> Self {
        DreamscrollAuthUser {
            id: claims.sub,
            session_hash: String::new(),
            claims: Some(claims),
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
        Ok(Verification::Success(DreamscrollAuthUser::from(db_user)))
    } else {
        Ok(Verification::InvalidPassword)
    }
}
