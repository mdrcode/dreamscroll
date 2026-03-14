use std::sync::Arc;

use crate::webhook::logic::{illuminate::IlluminationTask, spark::SparkTask};

use super::*;

#[derive(Clone, Default)]
pub struct Beacon {
    illumination_queue: Option<Arc<dyn TaskQueue<Task = IlluminationTask>>>,
    spark_queue: Option<Arc<dyn TaskQueue<Task = SparkTask>>>,
}

impl Beacon {
    pub fn builder() -> BeaconBuilder {
        BeaconBuilder::default()
    }

    pub async fn signal_new_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        let Some(queue) = self.illumination_queue.as_ref() else {
            tracing::warn!("New capture created but no topic configured, skipping enqueue.");
            return Ok(());
        };

        queue.enqueue(IlluminationTask { capture_id }).await.inspect_err(
            |err| tracing::error!(queue = ?queue, capture_id, error = ?err, "Failed to enqueue capture for illumination: {}", err),
        )?;

        Ok(())
    }

    pub async fn signal_new_spark(&self, capture_ids: Vec<i32>) -> anyhow::Result<()> {
        if capture_ids.is_empty() {
            anyhow::bail!("signal_new_spark requires at least one capture_id");
        }

        let Some(queue) = self.spark_queue.as_ref() else {
            tracing::warn!(
                capture_ids = ?capture_ids,
                "Spark requested but no spark queue configured, skipping enqueue."
            );
            return Ok(());
        };

        queue
            .enqueue(SparkTask {
                capture_ids: capture_ids.clone(),
            })
            .await
            .inspect_err(|err| {
                tracing::error!(
                    queue = ?queue,
                    capture_ids = ?capture_ids,
                    error = ?err,
                    "Failed to enqueue captures for spark: {}",
                    err
                )
            })?;

        Ok(())
    }
}

#[derive(Default)]
pub struct BeaconBuilder {
    illumination_queue: Option<Arc<dyn TaskQueue<Task = IlluminationTask>>>,
    spark_queue: Option<Arc<dyn TaskQueue<Task = SparkTask>>>,
}

impl BeaconBuilder {
    pub fn illumination_queue(
        mut self,
        illumination_queue: impl TaskQueue<Task = IlluminationTask> + 'static,
    ) -> Self {
        self.illumination_queue = Some(Arc::new(illumination_queue));
        self
    }

    pub fn build(self) -> Beacon {
        Beacon {
            illumination_queue: self.illumination_queue,
            spark_queue: self.spark_queue,
        }
    }

    pub fn spark_queue(mut self, spark_queue: impl TaskQueue<Task = SparkTask> + 'static) -> Self {
        self.spark_queue = Some(Arc::new(spark_queue));
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    //use crate::webhook::r_wh_illuminate::IlluminationTask;

    use super::*;

    #[derive(Debug, Clone)]
    struct RecordingQueue {
        captures: Arc<Mutex<Vec<i32>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl TaskQueue for RecordingQueue {
        type Task = IlluminationTask;

        async fn enqueue(&self, task: Self::Task) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("enqueue failed")
            }

            let mut captures = self
                .captures
                .lock()
                .expect("RecordingQueue captures mutex should not be poisoned");
            captures.push(task.capture_id);
            Ok(())
        }

        async fn get_status(&self, _task_id: &str) -> anyhow::Result<TaskStatus> {
            unimplemented!();
        }
    }

    #[tokio::test]
    async fn signal_new_capture_enqueues_task() {
        let captures = Arc::new(Mutex::new(Vec::new()));
        let queue = RecordingQueue {
            captures: Arc::clone(&captures),
            fail: false,
        };

        let beacon = Beacon::builder().illumination_queue(queue).build();

        beacon
            .signal_new_capture(42)
            .await
            .expect("signal_new_capture should succeed");

        let recorded = captures
            .lock()
            .expect("captures mutex should not be poisoned")
            .clone();
        assert_eq!(recorded, vec![42]);
    }

    #[tokio::test]
    async fn signal_new_capture_without_queue_is_noop() {
        let beacon = Beacon::default();

        beacon
            .signal_new_capture(7)
            .await
            .expect("signal_new_capture should be a no-op when queue is absent");
    }

    #[tokio::test]
    async fn signal_new_capture_propagates_enqueue_error() {
        let queue = RecordingQueue {
            captures: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        };
        let beacon = Beacon::builder().illumination_queue(queue).build();

        let result = beacon.signal_new_capture(9).await;
        assert!(result.is_err());
    }
}
