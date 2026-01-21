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
