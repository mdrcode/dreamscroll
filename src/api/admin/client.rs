use anyhow::anyhow;
use std::sync::Arc;

use crate::{api::*, auth, backfill, config, database, stats, task};

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

    pub async fn queue_stats(
        &self,
        context: &auth::Context,
        windows: &[i64],
        task_backend: config::TaskQueueBackend,
    ) -> Result<stats::QueueStatsResponse, ApiError> {
        ensure_admin(context)?;
        stats::queue_stats(&self.db, windows, task_backend)
            .await
            .map_err(ApiError::internal)
    }

    pub async fn queue_failures(
        &self,
        context: &auth::Context,
        hours: i64,
        limit: i64,
        include_retrying: bool,
    ) -> Result<stats::QueueFailuresResponse, ApiError> {
        ensure_admin(context)?;
        stats::queue_failures(&self.db, hours, limit, include_retrying)
            .await
            .map_err(ApiError::internal)
    }

    pub async fn inference_stats(
        &self,
        context: &auth::Context,
        windows: &[i64],
    ) -> Result<stats::InferenceStatsResponse, ApiError> {
        ensure_admin(context)?;
        stats::inference_stats(&self.db, windows)
            .await
            .map_err(ApiError::internal)
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
