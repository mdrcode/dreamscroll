use serde::{Deserialize, Serialize};

use crate::{api, ignition, task};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkTask {
    pub capture_ids: Vec<i32>,
}

impl task::TaskId for SparkTask {
    fn id(&self) -> String {
        self.capture_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("-")
    }
}

pub async fn exec(
    service_api: &api::ServiceApiClient,
    firestarter: &Box<dyn ignition::Firestarter>,
    task: SparkTask,
) -> Result<(), api::ApiError> {
    if task.capture_ids.is_empty() {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    tracing::debug!(capture_ids = ?task.capture_ids, "Spark Webhook: task capture IDs");

    let captures = service_api
        .get_captures(Some(task.capture_ids.clone()))
        .await?;
    if captures.is_empty() {
        tracing::warn!(capture_ids = ?task.capture_ids, "No captures found during spark inference");
        return Ok(()); // TODO should this return a 404 instead?
    }

    let user_id = captures[0].user_id;
    if captures.iter().any(|capture| capture.user_id != user_id) {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must belong to the same user"
        )));
    }

    let found_ids = captures.iter().map(|c| c.id).collect::<Vec<_>>();
    tracing::info!(found_ids = ?found_ids, "Spark Webhook: found capture ids");

    let spark = firestarter.spark(captures.clone()).await?;

    let referenced_ids = spark
        .clusters
        .iter()
        .flat_map(|cluster| cluster.capture_ids.clone())
        .collect::<Vec<_>>();
    tracing::info!(referenced_ids = ?referenced_ids, "Spark Webhook: inference referenced captures");

    service_api.insert_spark(user_id, spark).await?;

    tracing::info!(
        user_id,
        input_capture_ids = ?found_ids,
        referenced_capture_ids = ?referenced_ids,
        "Spark inference completed and inserted"
    );

    Ok(())
}
