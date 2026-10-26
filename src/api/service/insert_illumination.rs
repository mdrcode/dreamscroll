use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, TransactionTrait,
    sea_query::OnConflict,
};

use crate::{api::*, database::DbHandle, illumination::v2, model};

/// Upsert the relational illumination projection for one TaskRun.
pub async fn insert_illumination_v2(
    db: &DbHandle,
    capture: &CaptureInfo,
    task_run_id: &str,
    inference_meta_id: Option<i32>,
    illumination: v2::Illumination,
) -> Result<(), ApiError> {
    let search_content = format_for_search_v2(&illumination);
    let entities = illumination.entities;
    let suggested_searches = illumination.suggested_searches;
    let summary = illumination.summary;
    let details = illumination.details;
    let tx = db.conn.begin().await?;

    let active = model::illumination::ActiveModel {
        user_id: Set(capture.user_id),
        capture_id: Set(capture.id),
        task_run_id: Set(task_run_id.to_owned()),
        inference_meta_id: Set(inference_meta_id),
        summary: Set(summary),
        details: Set(details),
        suggested_searches: Set(Some(suggested_searches)),
        ..Default::default()
    };
    model::illumination::Entity::insert(active)
        .on_conflict(
            OnConflict::column(model::illumination::Column::TaskRunId)
                .update_columns([
                    model::illumination::Column::UserId,
                    model::illumination::Column::CaptureId,
                    model::illumination::Column::InferenceMetaId,
                    model::illumination::Column::Summary,
                    model::illumination::Column::Details,
                    model::illumination::Column::SuggestedSearches,
                ])
                .to_owned(),
        )
        .exec(&tx)
        .await?;

    let illumination_row = model::illumination::Entity::find()
        .filter(model::illumination::Column::TaskRunId.eq(task_run_id))
        .one(&tx)
        .await?
        .ok_or_else(|| {
            ApiError::internal(anyhow::anyhow!("Illumination upsert returned no row"))
        })?;

    model::knowledge_entity::Entity::delete_many()
        .filter(model::knowledge_entity::Column::IlluminationId.eq(illumination_row.id))
        .exec(&tx)
        .await?;
    model::search_index::Entity::delete_many()
        .filter(model::search_index::Column::IlluminationId.eq(illumination_row.id))
        .exec(&tx)
        .await?;

    for (position, entity) in entities.iter().enumerate() {
        let (platform, handle, display_name) = match &entity.platform_link {
            Some(link) => (
                Some(link.platform.to_string()),
                link.handle.clone(),
                link.display_name.clone(),
            ),
            None => (None, None, None),
        };
        model::knowledge_entity::ActiveModel::builder()
            .set_user_id(capture.user_id)
            .set_capture_id(capture.id)
            .set_illumination_id(illumination_row.id)
            .set_position(position as i32)
            .set_name(&entity.name)
            .set_entity_type(entity.entity_type.to_string())
            .set_description(entity.description.clone())
            .set_search_query(entity.search_query.clone())
            .set_platform(platform)
            .set_platform_handle(handle)
            .set_platform_display_name(display_name)
            .insert(&tx)
            .await?;
    }

    model::search_index::ActiveModel::builder()
        .set_user_id(capture.user_id)
        .set_capture_id(capture.id)
        .set_illumination_id(illumination_row.id)
        .set_content(search_content)
        .insert(&tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Upsert one raw illumination result per TaskRun; retries replace its content.
pub async fn insert_illumination_raw(
    db: &DbHandle,
    capture: &CaptureInfo,
    task_run_id: &str,
    inference_meta_id: Option<i32>,
    raw: &serde_json::Value,
) -> Result<(), ApiError> {
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
    let active = model::illumination_raw::ActiveModel {
        user_id: Set(capture.user_id),
        capture_id: Set(capture.id),
        media_id: Set(media_id),
        task_run_id: Set(task_run_id.to_owned()),
        inference_meta_id: Set(inference_meta_id),
        content: Set(raw.clone()),
        ..Default::default()
    };
    model::illumination_raw::Entity::insert(active)
        .on_conflict(
            OnConflict::column(model::illumination_raw::Column::TaskRunId)
                .update_columns([
                    model::illumination_raw::Column::UserId,
                    model::illumination_raw::Column::CaptureId,
                    model::illumination_raw::Column::MediaId,
                    model::illumination_raw::Column::InferenceMetaId,
                    model::illumination_raw::Column::Content,
                ])
                .to_owned(),
        )
        .exec(&db.conn)
        .await?;

    Ok(())
}

pub fn format_for_search_v2(illumination: &v2::Illumination) -> String {
    illumination.to_markdown()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::illumination::v1;
    use crate::llms::InferenceMetadata;
    use chrono::Utc;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use serde_json::json;

    async fn record_usage(db: &DbHandle, user_id: i32, run_id: &str, tokens: i64) -> i32 {
        super::super::record_inference(
            db,
            user_id,
            run_id,
            1,
            "illuminate",
            InferenceMetadata {
                provider_name: "gemini".to_owned(),
                backend_name: "vertex".to_owned(),
                model_id: "test-model".to_owned(),
                tokens_in: Some(tokens),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_persists_v1_raw_only_and_v2_projection() {
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
            archived_at: None,
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
        let v1_task_run_id = uuid::Uuid::new_v4().to_string();
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

        let illumination = v1::Illumination::from_raw_json(content.clone()).unwrap();
        assert_eq!(illumination.raw_json(), &content);

        let v1_meta_id = record_usage(&db, user.id, &v1_task_run_id, 12).await;
        insert_illumination_raw(
            &db,
            &capture_info,
            &v1_task_run_id,
            Some(v1_meta_id),
            illumination.raw_json(),
        )
        .await
        .unwrap();

        let stored_raw = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::TaskRunId.eq(&v1_task_run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_raw.inference_meta_id, Some(v1_meta_id));
        assert_eq!(stored_raw.media_id, media.id);
        assert_eq!(stored_raw.task_run_id, v1_task_run_id);
        assert_eq!(stored_raw.content, content);

        let v1_parent_rows = model::illumination::Entity::find()
            .filter(model::illumination::Column::CaptureId.eq(capture.id))
            .all(&db.conn)
            .await
            .unwrap();
        assert!(v1_parent_rows.is_empty());
        let v1_entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::UserId.eq(user.id))
            .all(&db.conn)
            .await
            .unwrap();
        assert!(v1_entities.is_empty());
        let v1_search_indexes = model::search_index::Entity::find()
            .filter(model::search_index::Column::UserId.eq(user.id))
            .all(&db.conn)
            .await
            .unwrap();
        assert!(v1_search_indexes.is_empty());
        let v2_content = json!({
            "summary": "A Reddit community for NFC West memes",
            "details": "A community focused on memes about the NFL's NFC West division.",
            "suggested_searches": ["NFC West memes", "Arizona Cardinals meme history"],
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
        let v2_task_run_id = uuid::Uuid::new_v4().to_string();
        let v2_result = crate::illumination::v2::Illumination::from_raw_json(v2_content.clone())
            .expect("v2 output should deserialize");
        let v2_meta_id = record_usage(&db, user.id, &v2_task_run_id, 20).await;
        insert_illumination_raw(
            &db,
            &capture_info,
            &v2_task_run_id,
            Some(v2_meta_id),
            v2_result.raw_json(),
        )
        .await
        .unwrap();
        insert_illumination_v2(
            &db,
            &capture_info,
            &v2_task_run_id,
            Some(v2_meta_id),
            v2_result,
        )
        .await
        .unwrap();

        let stored_v2 = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::CaptureId.eq(capture.id))
            .filter(model::illumination_raw::Column::TaskRunId.eq(&v2_task_run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_v2.inference_meta_id, Some(v2_meta_id));
        assert_eq!(stored_v2.content, v2_content);
        assert_eq!(stored_v2.task_run_id, v2_task_run_id);
        assert_eq!(stored_v2.media_id, media.id);
        let stored_v2_illumination = model::illumination::Entity::find()
            .filter(model::illumination::Column::TaskRunId.eq(&v2_task_run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_v2_illumination.inference_meta_id, Some(v2_meta_id));
        assert_eq!(
            stored_v2_illumination.suggested_searches,
            Some(vec![
                "NFC West memes".to_string(),
                "Arizona Cardinals meme history".to_string(),
            ])
        );
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
        let retry_content = json!({
            "summary": "retry replaces the previous result",
            "details": "latest attempt output",
            "suggested_searches": [],
            "entities": []
        });
        let retry_result =
            crate::illumination::v2::Illumination::from_raw_json(retry_content.clone())
                .expect("retry output should deserialize");
        let retry_meta_id = record_usage(&db, user.id, &v2_task_run_id, 30).await;
        insert_illumination_raw(
            &db,
            &capture_info,
            &v2_task_run_id,
            Some(retry_meta_id),
            retry_result.raw_json(),
        )
        .await
        .unwrap();
        let prior_projection = model::illumination::Entity::find_by_id(stored_v2_illumination.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prior_projection.inference_meta_id, Some(v2_meta_id));
        assert_eq!(
            prior_projection.summary,
            "A Reddit community for NFC West memes"
        );
        insert_illumination_v2(
            &db,
            &capture_info,
            &v2_task_run_id,
            Some(retry_meta_id),
            retry_result,
        )
        .await
        .unwrap();

        let raw_rows = model::illumination_raw::Entity::find()
            .filter(model::illumination_raw::Column::TaskRunId.eq(&v2_task_run_id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(raw_rows.len(), 1);
        assert_eq!(raw_rows[0].content, retry_content);
        assert_eq!(raw_rows[0].inference_meta_id, Some(retry_meta_id));
        let usage_rows = model::inference_meta::Entity::find()
            .filter(model::inference_meta::Column::TaskRunId.eq(&v2_task_run_id))
            .order_by_asc(model::inference_meta::Column::Id)
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(
            usage_rows
                .iter()
                .map(|row| (row.id, row.tokens_in))
                .collect::<Vec<_>>(),
            vec![(v2_meta_id, Some(20)), (retry_meta_id, Some(30))]
        );
        insert_illumination_raw(&db, &capture_info, &v2_task_run_id, None,
            &json!({
                "summary": "untracked output", "details": "metadata insert failed", "suggested_searches": [], "entities": []
            })).await.unwrap();
        let untracked = model::illumination_raw::Entity::find_by_id(raw_rows[0].id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(untracked.content["summary"], "untracked output");
        assert_eq!(untracked.inference_meta_id, None);

        let illumination_rows = model::illumination::Entity::find()
            .filter(model::illumination::Column::TaskRunId.eq(&v2_task_run_id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(illumination_rows.len(), 1);
        assert_eq!(illumination_rows[0].inference_meta_id, Some(retry_meta_id));
        assert_eq!(
            illumination_rows[0].summary,
            "retry replaces the previous result"
        );
        let retry_entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::IlluminationId.eq(illumination_rows[0].id))
            .all(&db.conn)
            .await
            .unwrap();
        assert!(retry_entities.is_empty());

        insert_illumination_v2(
            &db,
            &capture_info,
            &v2_task_run_id,
            None,
            crate::illumination::v2::Illumination::from_raw_json(json!({
                "summary": "untracked projection", "details": "usage unavailable",
                "suggested_searches": [], "entities": []
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let untracked_projection = model::illumination::Entity::find_by_id(illumination_rows[0].id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(untracked_projection.summary, "untracked projection");
        assert_eq!(untracked_projection.inference_meta_id, None);
    }
}
