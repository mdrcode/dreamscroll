use std::sync::Arc;

use anyhow::Context;
use google_cloud_tasks_v2::model::OidcToken;

use crate::config;
use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::webhook::localclient::LocalWebhookClient;

use super::*;

pub fn make_cloud_tasks_queue_path(project_id: &str, region: &str, queue_id: &str) -> String {
    format!("projects/{project_id}/locations/{region}/queues/{queue_id}")
}

pub fn make_prod_webhook_url(base_url: &str, queue_name: &str) -> String {
    format!(
        "{}/_wh/cloudtask/{}",
        base_url.trim_end_matches('/'),
        queue_name.trim_start_matches('/')
    )
}

pub async fn make_task_master(
    cfg: &config::Config,
    db: DbHandle,
) -> anyhow::Result<Arc<TaskMaster>> {
    match cfg.task_backend {
        config::TaskQueueBackend::Local => {
            let illuminate_queue = {
                let url = make_prod_webhook_url(
                    &cfg.task_webhook_base_url,
                    &cfg.task_queue_name_illuminate,
                );
                LocalTaskQueue::connect(4, move |task: TaskEnvelope<IlluminationTask>| {
                    let client = LocalWebhookClient::new();
                    let url = url.clone();
                    async move { client.post_task(&url, &task).await }
                })
            };

            let spark_queue = {
                let url =
                    make_prod_webhook_url(&cfg.task_webhook_base_url, &cfg.task_queue_name_spark);
                LocalTaskQueue::connect(4, move |task: TaskEnvelope<SparkTask>| {
                    let client = LocalWebhookClient::new();
                    let url = url.clone();
                    async move { client.post_task(&url, &task).await }
                })
            };

            let search_index_queue = {
                let url = make_prod_webhook_url(
                    &cfg.task_webhook_base_url,
                    &cfg.task_queue_name_search_index,
                );
                LocalTaskQueue::connect(4, move |task: TaskEnvelope<SearchIndexTask>| {
                    let client = LocalWebhookClient::new();
                    let url = url.clone();
                    async move { client.post_task(&url, &task).await }
                })
            };

            Ok(Arc::new(
                TaskMaster::builder()
                    .db(db)
                    .max_attempts(cfg.task_max_attempts)
                    .illuminate_queue(illuminate_queue)
                    .search_index_queue(search_index_queue)
                    .spark_queue(spark_queue)
                    .build()?,
            ))
        }
        config::TaskQueueBackend::GCloudTasks => {
            let oidc_token = make_webhook_oidc_token(cfg)?;

            let illuminate_queue = CloudTaskQueue::connect(
                make_cloud_tasks_queue_path(
                    &cfg.gcloud_project_id,
                    &cfg.gcloud_project_region,
                    &cfg.task_queue_name_illuminate,
                ),
                make_prod_webhook_url(&cfg.task_webhook_base_url, &cfg.task_queue_name_illuminate),
                oidc_token.clone(),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Illumination")?;

            let spark_queue = CloudTaskQueue::connect(
                make_cloud_tasks_queue_path(
                    &cfg.gcloud_project_id,
                    &cfg.gcloud_project_region,
                    &cfg.task_queue_name_spark,
                ),
                make_prod_webhook_url(&cfg.task_webhook_base_url, &cfg.task_queue_name_spark),
                oidc_token.clone(),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Spark")?;

            let search_index_queue = CloudTaskQueue::connect(
                make_cloud_tasks_queue_path(
                    &cfg.gcloud_project_id,
                    &cfg.gcloud_project_region,
                    &cfg.task_queue_name_search_index,
                ),
                make_prod_webhook_url(
                    &cfg.task_webhook_base_url,
                    &cfg.task_queue_name_search_index,
                ),
                oidc_token.clone(),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: SearchIndex")?;

            Ok(Arc::new(
                TaskMaster::builder()
                    .db(db)
                    .max_attempts(cfg.task_max_attempts)
                    .illuminate_queue(illuminate_queue)
                    .search_index_queue(search_index_queue)
                    .spark_queue(spark_queue)
                    .build()?,
            ))
        }
    }
}

fn make_webhook_oidc_token(cfg: &config::Config) -> Result<OidcToken, anyhow::Error> {
    let oidc_service_account_email = cfg
        .task_oidc_service_account_email
        .as_ref()
        .context("Cloud Tasks OIDC service account is not configured")?
        .clone();
    let oidc_audience = cfg
        .task_oidc_audience
        .as_ref()
        .context("Cloud Tasks OIDC audience is not configured")?
        .clone();
    Ok(OidcToken::new()
        .set_service_account_email(oidc_service_account_email)
        .set_audience(oidc_audience))
}
