use std::sync::Arc;

use axum_login::UserId;
use sea_orm::EntityTrait;
use serde::Deserialize;

use crate::{auth, database::DbHandle, entity::user};

use super::{AuthError, DreamscrollAuthUser};

#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone)]
pub struct WebAuthBackend {
    db: Arc<DbHandle>,
}

impl WebAuthBackend {
    pub fn new(db: Arc<DbHandle>) -> Self {
        Self { db }
    }
}

impl axum_login::AuthnBackend for WebAuthBackend {
    type User = DreamscrollAuthUser;
    type Credentials = Credentials;
    type Error = AuthError;

    async fn authenticate(
        &self,
        creds: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        let auth_user =
            auth::password::verify(&self.db, &creds.username, &creds.password).await?;

        Ok(Some(auth_user))
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        let auth_user = user::Entity::find_by_id(*user_id)
            .one(&self.db.conn)
            .await?
            .map(DreamscrollAuthUser::from_db_model);

        Ok(auth_user)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_deserialize() {
        // Test that Credentials can be deserialized from JSON
        let json = r#"{"username": "testuser", "password": "testpass"}"#;
        let creds: Credentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.username, "testuser");
        assert_eq!(creds.password, "testpass");
    }

    #[test]
    fn test_web_auth_backend_is_clone() {
        // Verify that WebAuthBackend implements Clone (required by axum-login)
        // This ensures the backend can be shared across request handlers
        fn assert_clone<T: Clone>() {}
        assert_clone::<WebAuthBackend>();
    }

    // Note: Full integration testing of authenticate() and get_user() would require:
    // 1. Setting up a test database with known user records
    // 2. Testing successful authentication with valid credentials
    // 3. Testing failed authentication with invalid credentials
    // 4. Testing session rehydration via get_user()
    // 
    // These tests are better suited for integration tests that can spin up
    // a full database and test the complete authentication flow end-to-end.
}