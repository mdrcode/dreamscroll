use axum_login::UserId;
use sea_orm::EntityTrait;
use serde::Deserialize;

use crate::{auth, database::DbHandle, model::user};

use super::{AuthError, DreamscrollAuthUser};

#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone)]
pub struct WebAuthBackend {
    db: DbHandle,
}

impl WebAuthBackend {
    pub fn new(db: DbHandle) -> Self {
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
        // axum-login convention:
        //   Ok(Some(user)) — authenticated successfully
        //   Ok(None)       — credentials rejected (wrong password / unknown user)
        //   Err(e)         — system failure (DB error, hash error, etc.)
        match auth::password::authenticate(&self.db, &creds.username, &creds.password).await {
            Ok(user) => Ok(Some(user)),
            Err(AuthError::InvalidCredentials) => Ok(None),
            Err(e) => Err(e),
        }
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
    use axum_login::AuthnBackend;

    use crate::test_support::db_or_skip;

    use super::*;

    #[tokio::test]
    async fn authenticate_maps_rejections_to_none_and_valid_credentials_to_the_user() {
        db_or_skip!(db);
        let username = format!("login_{}", uuid::Uuid::new_v4().simple());
        let inserted = user::ActiveModel::builder()
            .set_username(username.clone())
            .set_password_hash(auth::password::hash("correct horse").expect("hash should succeed"))
            .set_storage_shard("shard-1")
            .insert(&db.conn)
            .await
            .expect("user should insert");

        let backend = WebAuthBackend::new(db);
        let credentials = |password: &str| Credentials {
            username: username.clone(),
            password: password.to_string(),
        };

        let authenticated = backend
            .authenticate(credentials("correct horse"))
            .await
            .expect("a valid login is not a system error");
        assert_eq!(authenticated.map(|u| u.user_id()), Some(inserted.id));

        // Rejections are Ok(None), so axum-login answers 401 rather than 500 — for a wrong
        // password and an unknown user alike.
        assert!(backend
            .authenticate(credentials("wrong password"))
            .await
            .expect("a rejection is not a system error")
            .is_none());
        assert!(backend
            .authenticate(Credentials {
                username: "no_such_user".to_string(),
                password: "x".to_string(),
            })
            .await
            .expect("a rejection is not a system error")
            .is_none());
    }
}
