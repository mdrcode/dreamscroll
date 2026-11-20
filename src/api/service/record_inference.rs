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
        tokens_thought: Set(metadata.tokens_thought),
        params: Set(serde_json::Value::Object(metadata.params)),
        provider_metadata: Set(serde_json::Value::Object(metadata.provider_metadata)),
        raw_output: Set(metadata.raw_output),
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

#[cfg(test)]
mod tests {
    use sea_orm::EntityTrait;

    use super::*;

    #[tokio::test]
    async fn persists_raw_output_only_when_present() {
        crate::test_support::db_or_skip!(db);
        let user_id = crate::test_support::fixtures::user(&db, "raw_output").await;

        for (run_id, raw_output) in [
            ("run-with-raw", Some("{\"summary\":".to_string())),
            ("run-without-raw", None),
        ] {
            let id = record_inference(
                &db,
                user_id,
                run_id,
                1,
                "illuminate",
                llms::InferenceMetadata {
                    provider_name: "gemini".to_string(),
                    backend_name: "vertex".to_string(),
                    model_id: "test-model".to_string(),
                    raw_output: raw_output.clone(),
                    ..Default::default()
                },
            )
            .await
            .expect("metadata row should be written");

            let row = model::inference_meta::Entity::find_by_id(id)
                .one(&db.conn)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.raw_output, raw_output, "{run_id}");
        }
    }
}
