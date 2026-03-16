use anyhow::Context;

use crate::facility;
use crate::webhook::devclient::DevWebhookClient;
use crate::webhook::logic::{illuminate::IlluminationTask, spark::SparkTask};

use super::*;

pub async fn make_beacon(config: &facility::Config) -> anyhow::Result<Beacon> {
    match config.task_backend {
        TaskQueueBackend::Local => {
            let base_url = format!("http://localhost:{}", config.port);

            let dev_client_illuminate = DevWebhookClient::new(&base_url);
            let illumination_queue = LocalTaskQueue::connect(4, move |task: IlluminationTask| {
                let client = dev_client_illuminate.clone();
                async move { client.post_illuminate(&task).await }
            });

            let dev_client_spark = DevWebhookClient::new(&base_url);
            let spark_queue = LocalTaskQueue::connect(4, move |task: SparkTask| {
                let client = dev_client_spark.clone();
                async move { client.post_spark(&task).await }
            });

            Ok(Beacon::builder()
                .illumination_queue(illumination_queue)
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

            Ok(Beacon::builder()
                .illumination_queue(illumination_queue)
                .spark_queue(spark_queue)
                .build())
        }
    }
}
