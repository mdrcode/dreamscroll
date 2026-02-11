use anyhow::anyhow;

use crate::{api::*, auth, database};

pub struct AdminClient {
    db: database::DbHandle,
    admin_context: auth::Context,
}

impl AdminClient {
    pub fn new(db: database::DbHandle, admin_context: auth::Context) -> Result<Self, ApiError> {
        if !admin_context.is_admin() {
            return Err(ApiError::forbidden(anyhow!(
                "Only admin users can create new users"
            )));
        }

        Ok(Self { db, admin_context })
    }

    pub async fn create_user(
        &self,
        username: String,
        password: String,
        email: String,
    ) -> Result<UserInfo, ApiError> {
        super::create_user(&self.db, username, password, email).await
    }
}
