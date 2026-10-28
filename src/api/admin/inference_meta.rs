use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{api, database, model};

/// Every recorded inference attempt for a TaskRun, oldest attempt first.
pub(super) async fn get_inference_meta(
    db: &database::DbHandle,
    task_run_id: &str,
) -> Result<Vec<api::InferenceMetaInfo>, api::ApiError> {
    let rows = model::inference_meta::Entity::find()
        .filter(model::inference_meta::Column::TaskRunId.eq(task_run_id))
        .order_by_asc(model::inference_meta::Column::Attempt)
        .order_by_asc(model::inference_meta::Column::Id)
        .all(&db.conn)
        .await?;

    Ok(rows.into_iter().map(api::InferenceMetaInfo::from).collect())
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    use super::*;

    #[tokio::test]
    async fn returns_every_attempt_with_full_metadata() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = model::user::ActiveModel::builder()
            .set_username("meta-user")
            .set_password_hash("unused")
            .set_storage_shard("testshard")
            .insert(&db.conn)
            .await
            .unwrap();

        db.conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
INSERT INTO inference_metas
  (user_id, task_run_id, attempt, task_type, provider, backend, model,
   duration_ms, tokens_in, tokens_out, params, provider_metadata)
VALUES
  ($1, 'run-meta', 2, 'illuminate', 'gemini', 'vertex', 'test-model',
   2000, 200, 20, '{"prompt_version":"v2"}'::jsonb,
   '{"provider_request_id":"req_meta"}'::jsonb),
  ($1, 'run-meta', 1, 'illuminate', 'gemini', 'vertex', 'test-model',
   1000, 100, 10, '{}'::jsonb, '{}'::jsonb)
"#,
                vec![user.id.into()],
            ))
            .await
            .unwrap();

        let metas = get_inference_meta(&db, "run-meta").await.unwrap();

        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].attempt, Some(1));
        assert_eq!(metas[1].attempt, Some(2));
        assert_eq!(metas[1].params["prompt_version"], "v2");
        assert_eq!(
            metas[1].provider_metadata["provider_request_id"],
            "req_meta"
        );
        assert!(get_inference_meta(&db, "missing").await.unwrap().is_empty());
    }
}
