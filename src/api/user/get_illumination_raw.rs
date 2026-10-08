use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::ApiError, auth, database::DbHandle, model};

pub async fn get_illumination_raw(
    db: &DbHandle,
    context: &auth::Context,
    inference_id: &str,
) -> Result<serde_json::Value, ApiError> {
    let result = model::illumination_raw::Entity::find()
        .filter(model::illumination_raw::Column::UserId.eq(context.user_id()))
        .filter(model::illumination_raw::Column::InferenceId.eq(inference_id))
        .one(&db.conn)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(anyhow!(
                "Raw illumination result not found for ID {inference_id}"
            ))
        })?;

    Ok(result.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn raw_illumination_lookup_returns_exact_attempt_only_to_its_owner() {
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
        let first_inference_id = "00000000-0000-0000-0000-000000000001";
        let second_inference_id = "00000000-0000-0000-0000-000000000002";

        for (inference_id, content) in [
            (
                first_inference_id,
                serde_json::json!({"summary": "first inference"}),
            ),
            (
                second_inference_id,
                serde_json::json!({"summary": "second inference"}),
            ),
        ] {
            model::illumination_raw::ActiveModel::builder()
                .set_user_id(user.id)
                .set_capture_id(capture.id)
                .set_media_id(media.id)
                .set_inference_id(inference_id)
                .set_prompt_version("capture_illumination_v2")
                .set_provider_name("test")
                .set_backend_name("test")
                .set_model_id("test-model")
                .set_duration_ms(1)
                .set_provider_request_id(None)
                .set_provider_usage_json(None)
                .set_content(content)
                .insert(&db.conn)
                .await
                .unwrap();
        }

        let owner = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user.id));
        let first_result = get_illumination_raw(&db, &owner, first_inference_id)
            .await
            .unwrap();
        assert_eq!(
            first_result,
            serde_json::json!({"summary": "first inference"})
        );
        let result = get_illumination_raw(&db, &owner, second_inference_id)
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!({"summary": "second inference"}));

        let other_user = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(-1));
        let error = get_illumination_raw(&db, &other_user, second_inference_id)
            .await
            .unwrap_err();
        assert_eq!(error.status_code, axum::http::StatusCode::NOT_FOUND);
    }
}
