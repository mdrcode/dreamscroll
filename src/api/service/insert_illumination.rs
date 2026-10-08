use crate::{
    api::*, database::DbHandle, illumination::v1::Illumination, llms::InferenceResult, model,
};

pub async fn insert_illumination_v1(
    db: &DbHandle,
    capture: &CaptureInfo,
    illumination: Illumination,
) -> Result<(), ApiError> {
    let mut builder = model::illumination::ActiveModel::builder()
        .set_user_id(capture.user_id)
        .set_capture_id(capture.id)
        .set_summary(&illumination.summary)
        .set_details(&illumination.details)
        .set_search_index(
            model::search_index::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_content(format_for_search(&illumination)),
        );

    for entity in &illumination.entities {
        builder.knodes.push(
            model::knode::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_name(&entity.name)
                .set_description(&entity.description)
                .set_k_type(entity.entity_type.to_string()),
        );
    }

    for xquery in &illumination.suggested_searches {
        builder.xqueries.push(
            model::xquery::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_query(xquery),
        );
    }

    for sm in &illumination.social_media_accounts {
        builder.social_medias.push(
            model::social_media::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_display_name(&sm.display_name)
                .set_handle(&sm.handle)
                .set_platform(sm.platform.to_string()),
        );
    }

    builder.save(&db.conn).await?;

    Ok(())
}

pub async fn insert_illumination_raw<R: InferenceResult + ?Sized>(
    db: &DbHandle,
    capture: &CaptureInfo,
    raw: &R,
) -> Result<(), ApiError> {
    let metadata = raw.metadata();
    // Must match the versioned illuminate functions, which analyze the first media on the capture.
    let media_id = capture
        .medias
        .first()
        .map(|media| media.id)
        .ok_or_else(|| {
            ApiError::internal(anyhow::anyhow!(
                "Capture {} has no media for illumination inference",
                capture.id
            ))
        })?;
    model::illumination_raw::ActiveModel::builder()
        .set_user_id(capture.user_id)
        .set_capture_id(capture.id)
        .set_media_id(media_id)
        .set_inference_run_id(&metadata.inference_run_id)
        .set_prompt_version(&metadata.prompt_version)
        .set_provider_name(&metadata.provider_name)
        .set_backend_name(&metadata.backend_name)
        .set_model_id(&metadata.model_id)
        .set_duration_ms(metadata.duration_ms)
        .set_provider_request_id(metadata.provider_request_id.clone())
        .set_provider_usage_json(metadata.provider_usage_json.clone())
        .set_content(raw.raw_json().clone())
        .insert(&db.conn)
        .await?;

    Ok(())
}

pub fn format_for_search(illumination: &Illumination) -> String {
    // lol, a naive approach for now
    format!(
        "{} {} {} {} {}",
        illumination.summary,
        illumination.details,
        illumination
            .entities
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<String>>()
            .join(" "),
        illumination.suggested_searches.to_vec().join(" "),
        illumination
            .social_media_accounts
            .iter()
            .map(|s| s.display_name.clone())
            .collect::<Vec<String>>()
            .join(" ")
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llms::InferenceMetadata;
    use chrono::Utc;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use serde_json::json;

    #[tokio::test]
    async fn insert_persists_raw_results_and_v1_projection() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let user = model::user::ActiveModel::builder()
            .set_username(format!("raw_{}", uuid::Uuid::new_v4().simple()))
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
            .set_bytes(4)
            .set_mime_type(Some("image/jpeg".to_string()))
            .set_hash_blake3(Some("test-hash".to_string()))
            .set_storage_provider("test")
            .set_storage_user_shard(user.storage_shard.clone())
            .set_storage_uuid(uuid::Uuid::new_v4())
            .insert(&db.conn)
            .await
            .unwrap();
        let capture_info = CaptureInfo {
            id: capture.id,
            user_id: user.id,
            created_at: Utc::now(),
            created_at_human: String::new(),
            medias: vec![crate::api::MediaInfo {
                id: media.id,
                storage_uuid: media.storage_uuid,
                url: String::new(),
                mime_type: media.mime_type.clone(),
                hash_blake3: media.hash_blake3.clone(),
                storage_provider: media.storage_provider.clone(),
                storage_bucket: media.storage_bucket.clone(),
                storage_shard: media.storage_user_shard.clone(),
                storage_extension: media.storage_extension.clone(),
            }],
            illuminations: vec![],
            annotation: None,
        };
        let inference_run_id = format!(
            "u{}-illuminate-capture{}-run1-attempt1",
            user.id, capture.id
        );
        let content = json!({
            "summary": "A short summary",
            "details": "Detailed content",
            "suggested_searches": [],
            "entities": [],
            "social_media_accounts": [],
            "future_schema_field": {"retained": true}
        });

        let illumination = Illumination::from_raw_json(
            content.clone(),
            InferenceMetadata {
                inference_run_id: inference_run_id.clone(),
                prompt_version: "capture_illumination_v1".to_string(),
                provider_name: "gemini".to_string(),
                backend_name: "vertex".to_string(),
                model_id: "test-model".to_string(),
                duration_ms: 123,
                provider_request_id: Some("interaction-test".to_string()),
                provider_usage_json: Some(json!({"input_tokens": 12})),
            },
        )
        .unwrap();
        assert_eq!(illumination.raw_json(), &content);

        insert_illumination_raw(&db, &capture_info, &illumination)
            .await
            .unwrap();
        insert_illumination_v1(&db, &capture_info, illumination)
            .await
            .unwrap();

        let stored_raw = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::CaptureId.eq(capture.id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_raw.prompt_version, "capture_illumination_v1");
        assert_eq!(stored_raw.media_id, media.id);
        assert_eq!(stored_raw.inference_run_id, inference_run_id);
        assert_eq!(
            stored_raw.provider_request_id.as_deref(),
            Some("interaction-test")
        );
        assert_eq!(stored_raw.model_id, "test-model");
        assert_eq!(stored_raw.content, content);
        assert_eq!(
            stored_raw.provider_usage_json,
            Some(json!({"input_tokens": 12}))
        );

        let stored_illumination = model::illumination::Entity::find()
            .filter(model::illumination::Column::CaptureId.eq(capture.id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_illumination.summary, "A short summary");

        let v2_content = json!({
            "summary": "A Reddit community for NFC West memes",
            "details": "A community focused on memes about the NFL's NFC West division.",
            "suggested_searches": [],
            "entities": [{
                "name": "NFCWestMemeWar",
                "description": "A community for memes about the NFL's NFC West division.",
                "type": "online_community",
                "platform_link": {
                    "platform": "reddit",
                    "handle": "r/NFCWestMemeWar",
                    "url": "https://www.reddit.com/r/NFCWestMemeWar/"
                }
            }]
        });
        let v2_result = crate::illumination::v2::Illumination::from_raw_json(
            v2_content.clone(),
            InferenceMetadata {
                inference_run_id: inference_run_id.clone(),
                prompt_version: "capture_illumination_v2".to_string(),
                provider_name: "gemini".to_string(),
                backend_name: "vertex".to_string(),
                model_id: "test-model".to_string(),
                duration_ms: 100,
                provider_request_id: None,
                provider_usage_json: None,
            },
        )
        .expect("v2 output should deserialize");
        insert_illumination_raw(&db, &capture_info, &v2_result)
            .await
            .unwrap();

        let stored_v2 = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::CaptureId.eq(capture.id))
            .filter(model::illumination_raw::Column::PromptVersion.eq("capture_illumination_v2"))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_v2.prompt_version, "capture_illumination_v2");
        assert_eq!(stored_v2.content, v2_content);
        assert_eq!(stored_v2.inference_run_id, inference_run_id);
        assert_eq!(stored_v2.media_id, media.id);
    }
}
