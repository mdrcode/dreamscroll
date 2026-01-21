use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

use crate::{database::DbHandle, entity::user};

use super::{WebAuthUser, webautherror::*};

pub fn hash_password(password: &str) -> Result<String, WebAuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = PasswordHash::generate(argon2, password.as_bytes(), &salt)?.to_string();
    Ok(password_hash)
}

/// Verifies a password against a stored hash.
///
/// Returns Ok(true) if the password matches, Ok(false) if it doesn't,
/// or an error if the hash is malformed.
pub fn verify(hash: &str, password: &str) -> Result<bool, WebAuthError> {
    tracing::warn!("verify function is deprecated; use verify_password instead");
    let parsed_hash = PasswordHash::new(hash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

pub enum Verification {
    Success(WebAuthUser),
    NoSuchUser,
    InvalidPassword,
}

pub async fn verify_password(db: &DbHandle, u: &str, p: &str) -> Result<Verification, WebAuthError> {
    let db_user = match user::Entity::find_by_username(u).one(&db.conn).await? {
        Some(user) => user,
        None => return Ok(Verification::NoSuchUser),
    };

    let parsed_hash = PasswordHash::new(&db_user.password_hash)?;

    if Argon2::default()
        .verify_password(p.as_bytes(), &parsed_hash)
        .is_ok()
    {
        Ok(Verification::Success(WebAuthUser::from(db_user)))
    } else {
        Ok(Verification::InvalidPassword)
    }
}
