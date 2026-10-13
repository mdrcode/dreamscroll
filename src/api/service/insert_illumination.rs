use crate::{
    api::*,
    database::DbHandle,
    illumination::{KnowledgeEntityType, v1, v2},
    llms::InferenceResult,
    model,
};

pub async fn insert_illumination_v1(
    db: &DbHandle,
    capture: &CaptureInfo,
    illumination: v1::Illumination,
) -> Result<(), ApiError> {
    let inference_id = illumination.metadata().inference_id.clone();
    let mut builder = model::illumination::ActiveModel::builder()
        .set_user_id(capture.user_id)
        .set_capture_id(capture.id)
        .set_inference_id(Some(inference_id))
        .set_summary(&illumination.summary)
        .set_details(&illumination.details)
        .set_search_index(
            model::search_index::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_content(format_for_search(&illumination)),
        );

    let mut position = 0;
    for entity in &illumination.entities {
        builder.knowledge_entities.push(
            model::knowledge_entity::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_position(position)
                .set_name(&entity.name)
                .set_entity_type(entity.entity_type.to_string())
                .set_description(Some(entity.description.clone())),
        );
        position += 1;
    }

    for xquery in &illumination.suggested_searches {
        builder.xqueries.push(
            model::xquery::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_query(xquery),
        );
    }

    for account in &illumination.social_media_accounts {
        builder.knowledge_entities.push(
            model::knowledge_entity::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_position(position)
                .set_name(&account.display_name)
                .set_entity_type(KnowledgeEntityType::SocialMediaAccount.to_string())
                .set_platform(Some(account.platform.to_string()))
                .set_platform_handle(Some(account.handle.clone()))
                .set_platform_display_name(Some(account.display_name.clone())),
        );
        position += 1;
    }

    builder.save(&db.conn).await?;
    Ok(())
}

pub async fn insert_illumination_v2(
    db: &DbHandle,
    capture: &CaptureInfo,
    illumination: v2::Illumination,
) -> Result<(), ApiError> {
    let inference_id = illumination.metadata().inference_id.clone();
    let mut builder = model::illumination::ActiveModel::builder()
        .set_user_id(capture.user_id)
        .set_capture_id(capture.id)
        .set_inference_id(Some(inference_id))
        .set_summary(&illumination.summary)
        .set_details(&illumination.details)
        .set_search_index(
            model::search_index::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_content(format_for_search_v2(&illumination)),
        );

    for (position, entity) in illumination.entities.iter().enumerate() {
        let (platform, handle, display_name) = match &entity.platform_link {
            Some(link) => (
                Some(link.platform.to_string()),
                link.handle.clone(),
                link.display_name.clone(),
            ),
            None => (None, None, None),
        };
        builder.knowledge_entities.push(
            model::knowledge_entity::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_position(position as i32)
                .set_name(&entity.name)
                .set_entity_type(entity.entity_type.to_string())
                .set_description(entity.description.clone())
                .set_search_query(entity.search_query.clone())
                .set_platform(platform)
                .set_platform_handle(handle)
                .set_platform_display_name(display_name),
        );
    }

    for search in &illumination.suggested_searches {
        builder.xqueries.push(
            model::xquery::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_capture_id(capture.id)
                .set_query(search),
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
        .set_inference_id(&metadata.inference_id)
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

pub fn format_for_search(illumination: &v1::Illumination) -> String {
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
pub fn format_for_search_v2(illumination: &v2::Illumination) -> String {
    illumination.to_markdown()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llms::InferenceMetadata;
    use chrono::Utc;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use serde_json::json;

    #[tokio::test]
    async fn insert_persists_raw_results_and_unified_entities_for_v1_and_v2() {
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
        let inference_id = uuid::Uuid::new_v4().to_string();
        let content = json!({
            "summary": "A short summary",
            "details": "Detailed content",
            "suggested_searches": [],
            "entities": [{
                "name": "Ada Lovelace",
                "description": "Pioneer of computing",
                "type": "real_person"
            }],
            "social_media_accounts": [{
                "display_name": "Ada",
                "handle": "@ada",
                "platform": "x_twitter"
            }],
            "future_schema_field": {"retained": true}
        });

        let illumination = v1::Illumination::from_raw_json(
            content.clone(),
            InferenceMetadata {
                inference_id: inference_id.clone(),
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
        assert_eq!(stored_raw.inference_id, inference_id);
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
        assert_eq!(
            stored_illumination.inference_id.as_deref(),
            Some(inference_id.as_str())
        );
        let v1_entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::IlluminationId.eq(stored_illumination.id))
            .order_by_asc(model::knowledge_entity::Column::Position)
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(v1_entities.len(), 2);
        assert_eq!(v1_entities[0].position, 0);
        assert_eq!(v1_entities[0].entity_type, "real_person");
        assert_eq!(
            v1_entities[0].description.as_deref(),
            Some("Pioneer of computing")
        );
        assert_eq!(v1_entities[1].position, 1);
        assert_eq!(v1_entities[1].entity_type, "social_media_account");
        assert_eq!(v1_entities[1].platform.as_deref(), Some("x_twitter"));
        assert_eq!(v1_entities[1].platform_handle.as_deref(), Some("@ada"));

        let v2_content = json!({
            "summary": "A Reddit community for NFC West memes",
            "details": "A community focused on memes about the NFL's NFC West division.",
            "suggested_searches": [],
            "entities": [{
                "name": "NFCWestMemeWar",
                "description": "A community for memes about the NFL's NFC West division.",
                "search_query": "NFCWestMemeWar subreddit",
                "type": "online_community",
                "platform_link": {
                    "platform": "reddit",
                    "display_name": "NFC West Meme War",
                    "handle": "r/NFCWestMemeWar",
                    "url": "https://www.reddit.com/r/NFCWestMemeWar/"
                }
            }]
        });
        let v2_inference_id = uuid::Uuid::new_v4().to_string();
        let v2_result = crate::illumination::v2::Illumination::from_raw_json(
            v2_content.clone(),
            InferenceMetadata {
                inference_id: v2_inference_id.clone(),
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
        insert_illumination_v2(&db, &capture_info, v2_result)
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
        assert_eq!(stored_v2.inference_id, v2_inference_id);
        assert_eq!(stored_v2.media_id, media.id);
        let stored_v2_illumination = model::illumination::Entity::find()
            .filter(model::illumination::Column::InferenceId.eq(&v2_inference_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let v2_entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::IlluminationId.eq(stored_v2_illumination.id))
            .order_by_asc(model::knowledge_entity::Column::Position)
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(v2_entities.len(), 1);
        assert_eq!(v2_entities[0].entity_type, "online_community");
        assert_eq!(
            v2_entities[0].search_query.as_deref(),
            Some("NFCWestMemeWar subreddit")
        );
        assert_eq!(v2_entities[0].platform.as_deref(), Some("reddit"));
        assert_eq!(
            v2_entities[0].platform_handle.as_deref(),
            Some("r/NFCWestMemeWar")
        );
    }
}
