use anyhow::Context;

use crate::config;
use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::ingest::IngestTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::webhook::localclient::LocalWebhookClient;

use std::sync::Arc;

use super::*;

pub async fn make_task_master(
    cfg: &config::Config,
    db: DbHandle,
) -> anyhow::Result<Arc<TaskMaster>> {
    match cfg.task_backend {
        TaskQueueBackend::Local => {
            let base_url = format!("http://localhost:{}", cfg.port);

            let dev_client_illuminate = LocalWebhookClient::new(&base_url);
            let illumination_queue =
                LocalTaskQueue::connect(4, move |task: TaskEnvelope<IlluminationTask>| {
                    let client = dev_client_illuminate.clone();
                    async move { client.post_task("/_wh/cloudtask/illuminate", &task).await }
                });

            let dev_client_ingest = LocalWebhookClient::new(&base_url);
            let ingest_queue = LocalTaskQueue::connect(4, move |task: TaskEnvelope<IngestTask>| {
                let client = dev_client_ingest.clone();
                async move { client.post_task("/_wh/cloudtask/ingest", &task).await }
            });

            let dev_client_spark = LocalWebhookClient::new(&base_url);
            let spark_queue = LocalTaskQueue::connect(4, move |task: TaskEnvelope<SparkTask>| {
                let client = dev_client_spark.clone();
                async move { client.post_task("/_wh/cloudtask/spark", &task).await }
            });

            let dev_client_search_index = LocalWebhookClient::new(&base_url);
            let search_index_queue =
                LocalTaskQueue::connect(4, move |task: TaskEnvelope<SearchIndexTask>| {
                    let client = dev_client_search_index.clone();
                    async move { client.post_task("/_wh/cloudtask/search_index", &task).await }
                });

            Ok(Arc::new(
                TaskMaster::builder()
                    .db(db)
                    .ingest_queue(ingest_queue)
                    .illumination_queue(illumination_queue)
                    .search_index_queue(search_index_queue)
                    .spark_queue(spark_queue)
                    .build(),
            ))
        }
        TaskQueueBackend::GCloudTasks => {
            let illumination_queue = CloudTaskQueue::connect(
                cfg.gcloud_project_id.as_str(),
                cfg.gcloud_project_region.as_str(),
                cfg.task_cloudtask_queue_illumination
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_ILLUMINATION not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Illumination")?;
            let ingest_queue = CloudTaskQueue::connect(
                cfg.gcloud_project_id.as_str(),
                cfg.gcloud_project_region.as_str(),
                cfg.task_cloudtask_queue_ingest
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_INGEST not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Ingest")?;
            let spark_queue = CloudTaskQueue::connect(
                cfg.gcloud_project_id.as_str(),
                cfg.gcloud_project_region.as_str(),
                cfg.task_cloudtask_queue_spark
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_SPARK not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Spark")?;
            let search_index_queue = CloudTaskQueue::connect(
                cfg.gcloud_project_id.as_str(),
                cfg.gcloud_project_region.as_str(),
                cfg.task_cloudtask_queue_search_index
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_SEARCH_INDEX not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: SearchIndex")?;

            Ok(Arc::new(
                TaskMaster::builder()
                    .db(db)
                    .ingest_queue(ingest_queue)
                    .illumination_queue(illumination_queue)
                    .search_index_queue(search_index_queue)
                    .spark_queue(spark_queue)
                    .build(),
            ))
        }
    }
}
