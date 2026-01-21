use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

use crate::{database::DbHandle, entity::user};

use super::{autherror::AuthError, authuser::DreamscrollAuthUser};

pub fn hash(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = PasswordHash::generate(argon2, password.as_bytes(), &salt)?.to_string();
    Ok(password_hash)
}

pub async fn verify(db: &DbHandle, u: &str, p: &str) -> Result<DreamscrollAuthUser, AuthError> {
    let db_user = match user::Entity::find_by_username(u).one(&db.conn).await? {
        Some(user) => user,
        None => return Err(AuthError::InvalidCredentials),
    };

    let parsed_hash = PasswordHash::new(&db_user.password_hash)?;

    if Argon2::default()
        .verify_password(p.as_bytes(), &parsed_hash)
        .is_ok()
    {
        Ok(DreamscrollAuthUser::from_db_model(db_user))
    } else {
        Err(AuthError::InvalidCredentials)
    }
}
