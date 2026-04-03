use anyhow::Context;

use crate::facility;
use crate::webhook::localclient::LocalWebhookClient;
use crate::webhook::schema::{IngestTask, IlluminationTask, SearchIndexTask, SparkTask};

use super::*;

pub async fn make_beacon(config: &facility::Config) -> anyhow::Result<Beacon> {
    match config.task_backend {
        TaskQueueBackend::Local => {
            let base_url = format!("http://localhost:{}", config.port);

            let dev_client_illuminate = LocalWebhookClient::new(&base_url);
            let illumination_queue = LocalTaskQueue::connect(4, move |task: IlluminationTask| {
                let client = dev_client_illuminate.clone();
                async move { client.post_task("/_wh/cloudtask/illuminate", &task).await }
            });

            let dev_client_ingest = LocalWebhookClient::new(&base_url);
            let ingest_queue = LocalTaskQueue::connect(4, move |task: IngestTask| {
                let client = dev_client_ingest.clone();
                async move { client.post_task("/_wh/cloudtask/ingest", &task).await }
            });

            let dev_client_spark = LocalWebhookClient::new(&base_url);
            let spark_queue = LocalTaskQueue::connect(4, move |task: SparkTask| {
                let client = dev_client_spark.clone();
                async move { client.post_task("/_wh/cloudtask/spark", &task).await }
            });

            let dev_client_search_index = LocalWebhookClient::new(&base_url);
            let search_index_queue = LocalTaskQueue::connect(4, move |task: SearchIndexTask| {
                let client = dev_client_search_index.clone();
                async move { client.post_task("/_wh/cloudtask/search_index", &task).await }
            });

            Ok(Beacon::builder()
                .ingest_queue(ingest_queue)
                .illumination_queue(illumination_queue)
                .search_index_queue(search_index_queue)
                .spark_queue(spark_queue)
                .build())
        }
        TaskQueueBackend::GCloudPubSub => {
            let emulator = config.task_pubsub_emulator.as_deref();
            let illumination_queue = PubSubTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config
                    .task_pubsub_topic_new_capture
                    .as_ref()
                    .expect("TASK_PUBSUB_TOPIC_NEW_CAPTURE not set"),
                emulator,
            )
            .await
            .context("Failed to initialize Pub/Sub queue: Illumination")?;
            let spark_queue = PubSubTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config
                    .task_pubsub_topic_spark
                    .as_ref()
                    .expect("TASK_PUBSUB_TOPIC_SPARK not set"),
                emulator,
            )
            .await
            .context("Failed to initialize Pub/Sub queue: Spark")?;

            Ok(Beacon::builder()
                .illumination_queue(illumination_queue)
                .spark_queue(spark_queue)
                .build())
        }
        TaskQueueBackend::GCloudTasks => {
            let illumination_queue = CloudTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config.gcloud_project_region.as_str(),
                config
                    .task_cloudtask_queue_illumination
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_ILLUMINATION not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Illumination")?;
            let ingest_queue = CloudTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config.gcloud_project_region.as_str(),
                config
                    .task_cloudtask_queue_ingest
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_INGEST not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Ingest")?;
            let spark_queue = CloudTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config.gcloud_project_region.as_str(),
                config
                    .task_cloudtask_queue_spark
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_SPARK not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Spark")?;
            let search_index_queue = CloudTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config.gcloud_project_region.as_str(),
                config
                    .task_cloudtask_queue_search_index
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_SEARCH_INDEX not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: SearchIndex")?;

            Ok(Beacon::builder()
                .ingest_queue(ingest_queue)
                .illumination_queue(illumination_queue)
                .search_index_queue(search_index_queue)
                .spark_queue(spark_queue)
                .build())
        }
    }
}
