use crate::{api, ignition, webhook};

pub async fn exec(
    service_api: &api::ServiceApiClient,
    firestarter: &dyn ignition::Firestarter,
    task: webhook::schema::SparkTask,
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
        return Err(api::ApiError::not_found(anyhow::anyhow!(
            "No captures found for the provided capture IDs {:?}",
            task.capture_ids
        )));
    }

    let user_id = captures[0].user_id;
    if captures.iter().any(|capture| capture.user_id != user_id) {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "capture_ids must belong to the same user"
        )));
    }

    let found_ids = captures.iter().map(|c| c.id).collect::<Vec<_>>();
    tracing::debug!(found_ids = ?found_ids, "Spark Webhook: found capture ids");

    let spark_result = firestarter.spark(captures.clone()).await?;
    let spark_meta = spark_result.meta.clone();
    let spark = spark_result.spark;

    let referenced_ids = spark
        .clusters
        .iter()
        .flat_map(|cluster| cluster.capture_ids.clone())
        .collect::<Vec<_>>();

    service_api
        .insert_spark(user_id, task.capture_ids.clone(), spark, spark_meta)
        .await?;

    tracing::info!(
        user_id,
        input_capture_ids = ?task.capture_ids,
        found_capture_ids = ?found_ids,
        referenced_capture_ids = ?referenced_ids,
        "Spark inference completed and inserted"
    );

    Ok(())
}
