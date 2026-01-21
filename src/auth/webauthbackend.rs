use std::sync::Arc;

use axum_login::UserId;
use sea_orm::EntityTrait;
use serde::Deserialize;

use crate::{database::DbHandle, entity::user};

use super::{WebAuthError, WebAuthUser, password::*};

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
    type User = WebAuthUser;
    type Credentials = Credentials;
    type Error = WebAuthError;

    async fn authenticate(
        &self,
        creds: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        let verification = verify_password(&self.db, &creds.username, &creds.password).await?;

        match verification {
            Verification::Success(user) => Ok(Some(user)),
            _ => Ok(None),
        }
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        let auth_user = user::Entity::find_by_id(*user_id)
            .one(&self.db.conn)
            .await?
            .map(WebAuthUser::from);

        Ok(auth_user)
    }
}
