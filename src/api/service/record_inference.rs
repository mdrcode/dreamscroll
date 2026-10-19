use sea_orm::{ActiveModelTrait, ActiveValue::Set};

use crate::{database, llms, model};

/// Append usage independently of result writes; bookkeeping must not retry inference.
pub async fn record_inference(
    db: &database::DbHandle,
    user_id: i32,
    task_run_id: &str,
    attempt: i32,
    task_type: &str,
    metadata: llms::InferenceMetadata,
) -> Option<i32> {
    let result = model::inference_meta::ActiveModel {
        user_id: Set(user_id),
        task_run_id: Set(Some(task_run_id.to_owned())),
        attempt: Set(Some(attempt)),
        task_type: Set(Some(task_type.to_owned())),
        provider: Set(metadata.provider_name),
        backend: Set(metadata.backend_name),
        model: Set(metadata.model_id),
        duration_ms: Set(metadata.duration_ms),
        tokens_in: Set(metadata.tokens_in),
        tokens_in_cached: Set(metadata.tokens_in_cached),
        tokens_out: Set(metadata.tokens_out),
        params: Set(serde_json::Value::Object(metadata.params)),
        provider_metadata: Set(serde_json::Value::Object(metadata.provider_metadata)),
        ..Default::default()
    }
    .insert(&db.conn)
    .await;
    match result {
        Ok(row) => Some(row.id),
        Err(error) => {
            tracing::warn!(%error, task_run_id, attempt, task_type, "Failed to record inference metadata");
            None
        }
    }
}
