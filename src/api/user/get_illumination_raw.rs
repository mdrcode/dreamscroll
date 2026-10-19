use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::ApiError, auth, database::DbHandle, model};

pub async fn get_illumination_raw(
    db: &DbHandle,
    context: &auth::Context,
    task_run_id: &str,
) -> Result<serde_json::Value, ApiError> {
    let result = model::illumination_raw::Entity::find()
        .filter(model::illumination_raw::Column::UserId.eq(context.user_id()))
        .filter(model::illumination_raw::Column::TaskRunId.eq(task_run_id))
        .one(&db.conn)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(anyhow!(
                "Raw illumination result not found for TaskRun {task_run_id}"
            ))
        })?;

    Ok(result.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ignition::test_support::UnusedFirestarter,
        llms::test_support::TestInferenceClient,
        search::test_support::{UnusedEmbedder, UnusedVectorStore},
        storage::test_support::TestStorageProvider,
    };
    use chrono::Utc;

    #[tokio::test]
    async fn raw_illumination_lookup_returns_exact_task_run_only_to_its_owner() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = model::user::ActiveModel::builder()
            .set_username(format!(
                "illumination_raw_lookup_{}",
                uuid::Uuid::new_v4().simple()
            ))
            .set_password_hash("test-password-hash")
            .set_storage_shard(model::user::generate_storage_shard(8))
            .insert(&db.conn)
            .await
            .unwrap();
        let capture = model::capture::ActiveModel::builder()
            .set_user_id(user.id)
            .set_created_at(Utc::now())
            .insert(&db.conn)
            .await
            .unwrap();
        let media = model::media::ActiveModel::builder()
            .set_user_id(user.id)
            .set_capture_id(capture.id)
            .set_bytes(1)
            .set_mime_type(Some("image/jpeg".to_string()))
            .set_hash_blake3(None)
            .set_storage_provider("test")
            .set_storage_user_shard(user.storage_shard.clone())
            .set_storage_uuid(uuid::Uuid::new_v4())
            .insert(&db.conn)
            .await
            .unwrap();
        let first_task_run_id = "00000000-0000-0000-0000-000000000001";
        let second_task_run_id = "00000000-0000-0000-0000-000000000002";

        for (task_run_id, content) in [
            (
                first_task_run_id,
                serde_json::json!({"summary": "first inference"}),
            ),
            (
                second_task_run_id,
                serde_json::json!({"summary": "second inference"}),
            ),
        ] {
            model::illumination_raw::ActiveModel::builder()
                .set_user_id(user.id)
                .set_capture_id(capture.id)
                .set_media_id(media.id)
                .set_task_run_id(task_run_id)
                .set_inference_meta_id(None)
                .set_content(content)
                .insert(&db.conn)
                .await
                .unwrap();
        }

        let owner = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user.id));
        let first_result = get_illumination_raw(&db, &owner, first_task_run_id)
            .await
            .unwrap();
        assert_eq!(
            first_result,
            serde_json::json!({"summary": "first inference"})
        );
        let result = get_illumination_raw(&db, &owner, second_task_run_id)
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!({"summary": "second inference"}));

        let other_user = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(-1));
        let error = get_illumination_raw(&db, &other_user, second_task_run_id)
            .await
            .unwrap_err();
        assert_eq!(error.status_code, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn worker_result_is_retrieved_by_task_run_id() {
        use axum::response::IntoResponse;

        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = model::user::ActiveModel::builder()
            .set_username(format!(
                "illumination_e2e_{}",
                uuid::Uuid::new_v4().simple()
            ))
            .set_password_hash("test-password-hash")
            .set_storage_shard(model::user::generate_storage_shard(8))
            .insert(&db.conn)
            .await
            .unwrap();
        let capture = model::capture::ActiveModel::builder()
            .set_user_id(user.id)
            .set_created_at(Utc::now())
            .insert(&db.conn)
            .await
            .unwrap();
        model::media::ActiveModel::builder()
            .set_user_id(user.id)
            .set_capture_id(capture.id)
            .set_bytes(3)
            .set_mime_type(Some("image/jpeg".to_string()))
            .set_hash_blake3(None)
            .set_storage_provider("gcloud")
            .set_storage_bucket(Some("test-bucket".to_string()))
            .set_storage_user_shard(user.storage_shard.clone())
            .set_storage_uuid(uuid::Uuid::new_v4())
            .set_storage_extension(None)
            .insert(&db.conn)
            .await
            .unwrap();

        let config = crate::test_support::test_config::load().unwrap();
        let service_api = crate::api::ServiceApiClient::new(
            db.clone(),
            crate::storage::UrlMaker::from_config(&config),
        );
        let task_master = std::sync::Arc::new(
            crate::task::TaskMaster::builder()
                .db(db.clone())
                .build()
                .unwrap(),
        );
        let state = std::sync::Arc::new(crate::webhook::WebhookState {
            logic: crate::logic::LogicState {
                service_api,
                storage: Box::new(TestStorageProvider),
                inference_client: Box::new(TestInferenceClient::new(vec![
                    serde_json::json!({"summary": "missing required fields"}),
                    serde_json::json!({
                        "summary": "successful retry",
                        "details": "the second worker attempt succeeded",
                        "suggested_searches": [],
                        "entities": [],
                        "future_field": {"kept": true}
                    }),
                    serde_json::json!({
                        "summary": "intentional rerun",
                        "details": "a later run has its own result",
                        "suggested_searches": [],
                        "entities": []
                    }),
                ])),
                firestarter: Box::new(UnusedFirestarter),
                embedder: Box::new(UnusedEmbedder),
                vector_store: Box::new(UnusedVectorStore),
            },
            task_master: task_master.clone(),
        });
        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user.id));
        let task = crate::logic::illuminate::IlluminationTask::new(
            capture.id,
            user.id,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );

        let first_run = task_master
            .submit_illuminate(&context, task.clone(), Some("webhook-backfill"))
            .await
            .unwrap();
        let crate::task::SubmitOutcome::Enqueued {
            run_id: first_run_id,
            run_number: first_run_number,
        } = first_run
        else {
            panic!("first illumination run should enqueue");
        };
        let first_task_run = crate::task::TaskRun {
            logical_id: crate::task::TaskRun::make_logical_id(&task),
            run_id: first_run_id,
            run_number: first_run_number,
            backfill_id: Some("webhook-backfill".to_string()),
            task: task.clone(),
        };

        let failed_attempt = crate::webhook::r_illuminate::post(
            axum::extract::State(state.clone()),
            axum::Json(first_task_run.clone()),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(
            failed_attempt.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let failed_status = task_master
            .query_run_status(&context, &first_task_run.run_id)
            .await
            .unwrap()
            .unwrap();
        let failed_status = crate::api::TaskRunInfo::try_from(failed_status).unwrap();
        assert_eq!(
            failed_status.status,
            crate::task::TaskRunStatus::ErrorWillRetry
        );
        assert_eq!(failed_status.attempts, 1);
        let failed_metadata = model::inference_meta::Entity::find()
            .filter(model::inference_meta::Column::TaskRunId.eq(&first_task_run.run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .expect("typed output errors must retain inference metadata");
        assert_eq!(failed_metadata.user_id, user.id);
        assert_eq!(failed_metadata.attempt, Some(1));
        assert_eq!(failed_metadata.provider, "test");
        assert_eq!(failed_metadata.backend, "test");
        assert_eq!(failed_metadata.model, "test-model");
        assert_eq!(
            failed_metadata.provider_metadata["provider_request_id"],
            serde_json::json!("test-inference-0")
        );
        assert_eq!(
            failed_metadata.params,
            serde_json::json!({"prompt_version": "capture_illumination_v2"})
        );

        let successful_attempt = crate::webhook::r_illuminate::post(
            axum::extract::State(state.clone()),
            axum::Json(first_task_run.clone()),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(
            successful_attempt.status(),
            axum::http::StatusCode::NO_CONTENT
        );
        let completed_status = task_master
            .query_run_status(&context, &first_task_run.run_id)
            .await
            .unwrap()
            .unwrap();
        let completed_status = crate::api::TaskRunInfo::try_from(completed_status).unwrap();
        assert_eq!(
            completed_status.status,
            crate::task::TaskRunStatus::CompleteSuccess
        );
        assert_eq!(completed_status.attempts, 2);
        let indexing_runs = task_master
            .query_latest_task_runs(&context, "capture", &[capture.id])
            .await
            .unwrap();
        let indexing = indexing_runs
            .iter()
            .find(|run| run.task_type == "search_index")
            .unwrap();
        assert_eq!(indexing.backfill_id.as_deref(), Some("webhook-backfill"));
        assert_eq!(indexing.run_number, 1);
        let first_raw = get_illumination_raw(&db, &context, &first_task_run.run_id)
            .await
            .unwrap();
        assert_eq!(
            first_raw,
            serde_json::json!({
                "summary": "successful retry",
                "details": "the second worker attempt succeeded",
                "suggested_searches": [],
                "entities": [],
                "future_field": {"kept": true}
            })
        );
        let metadata = model::inference_meta::Entity::find()
            .filter(model::inference_meta::Column::TaskRunId.eq(&first_task_run.run_id))
            .order_by_id_asc()
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(metadata.len(), 2, "worker retries append usage records");
        assert_eq!(metadata[0].id, failed_metadata.id);
        assert_eq!(metadata[1].attempt, Some(2));
        let stored_raw = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::TaskRunId.eq(&first_task_run.run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_raw.inference_meta_id, Some(metadata[1].id));

        let rerun = task_master
            .submit_illuminate(&context, task.clone(), None)
            .await
            .unwrap();
        let crate::task::SubmitOutcome::Enqueued {
            run_id: second_run_id,
            run_number: second_run_number,
        } = rerun
        else {
            panic!("settled TaskRun should enqueue a new run");
        };
        let second_task_run = crate::task::TaskRun {
            logical_id: crate::task::TaskRun::make_logical_id(&task),
            run_id: second_run_id,
            run_number: second_run_number,
            backfill_id: None,
            task,
        };
        let rerun_response = crate::webhook::r_illuminate::post(
            axum::extract::State(state),
            axum::Json(second_task_run.clone()),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(rerun_response.status(), axum::http::StatusCode::NO_CONTENT);
        let rerun_status = task_master
            .query_run_status(&context, &second_task_run.run_id)
            .await
            .unwrap()
            .unwrap();
        let rerun_status = crate::api::TaskRunInfo::try_from(rerun_status).unwrap();
        assert_eq!(
            rerun_status.status,
            crate::task::TaskRunStatus::CompleteSuccess
        );
        assert_eq!(
            get_illumination_raw(&db, &context, &second_task_run.run_id)
                .await
                .unwrap(),
            serde_json::json!({
                "summary": "intentional rerun",
                "details": "a later run has its own result",
                "suggested_searches": [],
                "entities": []
            })
        );
    }
}
