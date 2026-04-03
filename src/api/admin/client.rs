use anyhow::anyhow;

use crate::{api::*, auth, database, task};

pub struct AdminApiClient {
    db: database::DbHandle,
    service_api: ServiceApiClient,
    beacon: task::Beacon,
}

impl AdminApiClient {
    pub fn new(
        db: database::DbHandle,
        service_api: ServiceApiClient,
        beacon: task::Beacon,
    ) -> Self {
        Self {
            db,
            service_api,
            beacon,
        }
    }

    pub async fn create_user(
        &self,
        context: &auth::Context,
        username: String,
        password: String,
        email: String,
    ) -> Result<UserInfo, ApiError> {
        ensure_admin(context)?;
        super::create_user(&self.db, username, password, email).await
    }

    pub async fn enqueue_backfill(
        &self,
        context: &auth::Context,
        req: BackfillRequest,
    ) -> Result<BackfillResponse, ApiError> {
        ensure_admin(context)?;
        super::backfill::enqueue(&self.service_api, &self.beacon, req).await
    }
}

fn ensure_admin(context: &auth::Context) -> Result<(), ApiError> {
    if !context.is_admin() {
        return Err(ApiError::forbidden(anyhow!(
            "Only admin users can perform this operation"
        )));
    }

    Ok(())
}
