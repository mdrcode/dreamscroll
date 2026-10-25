use anyhow::anyhow;
use std::sync::Arc;

use crate::{api::*, auth, backfill, database, task};

pub struct AdminApiClient {
    db: database::DbHandle,
    task_master: Arc<task::TaskMaster>,
}

impl AdminApiClient {
    pub fn new(db: database::DbHandle, task_master: Arc<task::TaskMaster>) -> Self {
        Self { db, task_master }
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

    pub async fn backfill_pump(
        &self,
        context: &auth::Context,
        req: backfill::BackfillPumpRequest,
    ) -> Result<backfill::BackfillPumpResponse, ApiError> {
        ensure_admin(context)?;
        backfill::pump(&self.db, &self.task_master, context, req).await
    }

    pub async fn backfill_status(
        &self,
        context: &auth::Context,
        backfill_id: &str,
    ) -> Result<backfill::BackfillStatusResponse, ApiError> {
        ensure_admin(context)?;
        backfill::status(&self.db, backfill_id).await
    }

    pub async fn backfill_failures(
        &self,
        context: &auth::Context,
        backfill_id: &str,
    ) -> Result<backfill::BackfillFailuresResponse, ApiError> {
        ensure_admin(context)?;
        backfill::failures(&self.db, backfill_id).await
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
