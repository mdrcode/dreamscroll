use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

use crate::{database::DbHandle, model::user};

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
        None => {
            // Dummy hash verification to mitigate timing attacks that
            // could reveal valid usernames.
            const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
            let parsed = PasswordHash::new(DUMMY_HASH).unwrap();
            let _ = Argon2::default().verify_password(p.as_bytes(), &parsed);
            return Err(AuthError::InvalidCredentials);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_creates_valid_argon2_hash() {
        let password = "test_password_123";
        let hash_result = hash(password);

        assert!(hash_result.is_ok());
        let hash_str = hash_result.unwrap();

        // Argon2 hashes start with $argon2
        assert!(hash_str.starts_with("$argon2"));

        // Should be reasonably long (argon2 hashes are ~90+ chars)
        assert!(hash_str.len() > 80);
    }

    #[test]
    fn test_hash_produces_different_salts() {
        let password = "same_password";

        let hash1 = hash(password).unwrap();
        let hash2 = hash(password).unwrap();

        // Same password should produce different hashes due to different salts
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_and_verify_roundtrip() {
        let password = "correct_password";
        let hash_str = hash(password).unwrap();

        // Verify the password matches the hash
        let parsed = PasswordHash::new(&hash_str).unwrap();
        let verify_result = Argon2::default().verify_password(password.as_bytes(), &parsed);

        assert!(verify_result.is_ok());
    }

    #[test]
    fn test_wrong_password_fails_verification() {
        let correct_password = "correct_password";
        let wrong_password = "wrong_password";

        let hash_str = hash(correct_password).unwrap();
        let parsed = PasswordHash::new(&hash_str).unwrap();

        // Wrong password should fail verification
        let verify_result = Argon2::default().verify_password(wrong_password.as_bytes(), &parsed);
        assert!(verify_result.is_err());
    }
}
