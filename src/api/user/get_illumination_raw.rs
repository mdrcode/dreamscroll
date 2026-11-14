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
        ignition::test_support::UnusedFirestarter, llms::test_support::TestInferenceClient, logic, search::test_support::{UnusedEmbedder, UnusedVectorStore}, storage::test_support::TestStorageProvider,
    };

    #[tokio::test]
    async fn raw_illumination_lookup_returns_exact_task_run_only_to_its_owner() {
        crate::test_support::db_or_skip!(db);
        let user_id = crate::test_support::fixtures::user(&db, "illumination_raw").await;
        let capture_id = crate::test_support::fixtures::capture(&db, user_id).await;
        let media = model::media::ActiveModel::builder()
            .set_user_id(user_id)
            .set_capture_id(capture_id)
            .set_bytes(1)
            .set_mime_type(Some("image/jpeg".to_string()))
            .set_hash_blake3(None)
            .set_storage_provider("test")
            .set_storage_user_shard(model::user::generate_storage_shard(8))
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
                .set_user_id(user_id)
                .set_capture_id(capture_id)
                .set_media_id(media.id)
                .set_task_run_id(task_run_id)
                .set_inference_meta_id(None)
                .set_content(content)
                .insert(&db.conn)
                .await
                .unwrap();
        }

        let owner = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user_id));
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
        assert_eq!(error.http_status_code, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn worker_result_is_retrieved_by_task_run_id() {
        use axum::response::IntoResponse;

        crate::test_support::db_or_skip!(db);
        let user_id = crate::test_support::fixtures::user(&db, "illumination_e2e").await;
        let capture_id = crate::test_support::fixtures::capture(&db, user_id).await;
        model::media::ActiveModel::builder()
            .set_user_id(user_id)
            .set_capture_id(capture_id)
            .set_bytes(3)
            .set_mime_type(Some("image/jpeg".to_string()))
            .set_hash_blake3(None)
            .set_storage_provider("gcloud")
            .set_storage_bucket(Some("test-bucket".to_string()))
            .set_storage_user_shard(model::user::generate_storage_shard(8))
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
        let knowledge = crate::knowledge::KnowledgeGraph::with_heuristic_linking(&db);
        let identity_engine = crate::knowledge::identity::IdentityEngine::new(
            knowledge.clone(),
            Default::default(),
            vec![Box::new(crate::knowledge::identity::wikidata::Wikidata::new().unwrap())],
        )
        .unwrap();
        let state = std::sync::Arc::new(crate::webhook::WebhookState {
            logic: logic::LogicState {
                cache: crate::fetch::FetchCache::new(&db.conn),
                identity_engine,
                wikidata_allowance: crate::fetch::AllowanceConfig {
                    max_calls: 0,
                    min_interval: std::time::Duration::ZERO,
                },
                service_api,
                knowledge,
                storage: Box::new(TestStorageProvider),
                inference_client: Box::new(TestInferenceClient::new(vec![
                    serde_json::json!({"summary": "missing required fields"}),
                    serde_json::json!({
                        "summary": "successful retry",
                        "details": "the second worker attempt succeeded",
                        "suggested_searches": [],
                        "entities": [{"name": "Existing person", "type": "real_person"}],
                        "future_field": {"kept": true}
                    }),
                    serde_json::json!({
                        "summary": "intentional rerun",
                        "details": "a later run has its own result",
                        "suggested_searches": [],
                        "entities": [{"name": "Raw-only person", "type": "real_person"}]
                    }),
                ])),
                firestarter: Box::new(UnusedFirestarter),
                embedder: Box::new(UnusedEmbedder),
                vector_store: Box::new(UnusedVectorStore),
            },
            task_master: task_master.clone(),
        });
        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user_id));
        let task = logic::illuminate::IlluminationTask::new(
            capture_id,
            user_id,
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
        // Execute an older queued payload that omitted the new field, not just a
        // serde round trip: the worker must still persist and enqueue indexing.
        let mut legacy_payload = serde_json::to_value(&first_task_run).unwrap();
        legacy_payload["task"]
            .as_object_mut()
            .unwrap()
            .remove("persist_projection");
        let first_task_run: crate::task::TaskRun<logic::illuminate::IlluminationTask> =
            serde_json::from_value(legacy_payload).unwrap();
        assert!(first_task_run.task.persist_projection);

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
        assert_eq!(failed_metadata.user_id, user_id);
        assert_eq!(failed_metadata.attempt, Some(1));

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
            .query_latest_task_runs(&context, "capture", &[capture_id])
            .await
            .unwrap();
        let indexing = indexing_runs
            .iter()
            .find(|run| run.task_type == "search_index")
            .unwrap();
        assert_eq!(indexing.backfill_id.as_deref(), Some("webhook-backfill"));
        // Settle the existing index run so an accidental raw-only enqueue cannot
        // be hidden by TaskMaster's in-flight deduplication.
        let index_task = logic::search_index::SearchIndexTask {
            capture_id,
            entity_owner_id: user_id,
        };
        let index_run = crate::task::TaskRun {
            logical_id: crate::task::TaskRun::make_logical_id(&index_task),
            run_id: indexing.run_id.clone(),
            run_number: indexing.run_number,
            backfill_id: indexing.backfill_id.clone(),
            task: index_task,
        };
        let index_attempt = task_master
            .begin_attempt(&index_run)
            .await
            .unwrap()
            .unwrap();
        task_master
            .finish_attempt(&index_run, index_attempt, &Ok(()))
            .await
            .unwrap();
        let first_raw = get_illumination_raw(&db, &context, &first_task_run.run_id)
            .await
            .unwrap();
        assert_eq!(
            first_raw,
            serde_json::json!({
                "summary": "successful retry",
                "details": "the second worker attempt succeeded",
                "suggested_searches": [],
                "entities": [{"name": "Existing person", "type": "real_person"}],
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
        let previous_projections = model::illumination::Entity::find()
            .filter(model::illumination::Column::CaptureId.eq(capture_id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(previous_projections.len(), 1);
        assert_eq!(previous_projections[0].task_run_id, first_task_run.run_id);
        let previous_entities = model::user_observed_entity::Entity::find()
            .filter(model::user_observed_entity::Column::CaptureId.eq(capture_id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(previous_entities.len(), 1);
        let previous_index_runs = model::task_run::Entity::find()
            .filter(model::task_run::Column::EntityId.eq(capture_id))
            .filter(model::task_run::Column::TaskType.eq("search_index"))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(previous_index_runs.len(), 1);
        let mut task = task;
        task.persist_projection = false;

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
                "entities": [{"name": "Raw-only person", "type": "real_person"}]
            })
        );
        let raw_only_metadata = model::inference_meta::Entity::find()
            .filter(model::inference_meta::Column::TaskRunId.eq(&second_task_run.run_id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(raw_only_metadata.len(), 1);
        assert_eq!(raw_only_metadata[0].attempt, Some(1));
        let raw_only_row = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::TaskRunId.eq(&second_task_run.run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            raw_only_row.inference_meta_id,
            Some(raw_only_metadata[0].id)
        );
        assert_eq!(
            model::illumination::Entity::find()
                .filter(model::illumination::Column::CaptureId.eq(capture_id))
                .all(&db.conn)
                .await
                .unwrap(),
            previous_projections,
            "raw-only rerun must neither add nor replace older projections"
        );
        assert_eq!(
            model::user_observed_entity::Entity::find()
                .filter(model::user_observed_entity::Column::CaptureId.eq(capture_id))
                .all(&db.conn)
                .await
                .unwrap(),
            previous_entities,
            "raw-only entities must remain raw, preserving existing entities"
        );
        assert_eq!(
            model::task_run::Entity::find()
                .filter(model::task_run::Column::EntityId.eq(capture_id))
                .filter(model::task_run::Column::TaskType.eq("search_index"))
                .all(&db.conn)
                .await
                .unwrap(),
            previous_index_runs,
            "raw-only rerun must not enqueue indexing even when no index is in flight"
        );
    }
}
