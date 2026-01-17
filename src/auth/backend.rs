use std::sync::Arc;

use axum_login::{AuthnBackend, UserId};
use sea_orm::EntityTrait;
use serde::Deserialize;

use crate::{database::DbHandle, model::user};

use super::{autherror::*, password::*};

#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone)]
pub struct Backend {
    db: Arc<DbHandle>,
}

impl Backend {
    pub fn new(db: Arc<DbHandle>) -> Self {
        Self { db }
    }
}

impl AuthnBackend for Backend {
    type User = user::Model;
    type Credentials = Credentials;
    type Error = AuthError;

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
        user::Entity::find_by_id(*user_id)
            .one(&self.db.conn)
            .await
            .map_err(AuthError::from)
    }
}
