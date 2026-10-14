use std::collections::HashMap;

use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

// This is user-specific, so it takes a context and only returns captures belonging to that user.
pub async fn get_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Vec<i32>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    if ids.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    let loader = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(model::capture::Column::Id.is_in(ids.clone()));

    let captures = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::annotation::Entity)
        .with((model::illumination::Entity, model::knowledge_entity::Entity))
        .all(&db.conn)
        .await?;

    let mut captures_by_id: HashMap<i32, model::capture::ModelEx> = captures
        .into_iter()
        .map(|capture| (capture.id, capture))
        .collect();

    let ordered = ids
        .into_iter()
        .filter_map(|capture_id| captures_by_id.remove(&capture_id))
        .collect();

    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn capture_loading_accepts_legacy_null_suggested_searches() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = model::user::ActiveModel::builder()
            .set_username(format!("legacy_illum_{}", uuid::Uuid::new_v4().simple()))
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
        model::illumination::ActiveModel::builder()
            .set_user_id(user.id)
            .set_capture_id(capture.id)
            .set_summary("legacy summary")
            .set_details("")
            .set_suggested_searches(None)
            .insert(&db.conn)
            .await
            .unwrap();

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user.id));
        let captures = get_captures(&db, &context, vec![capture.id]).await.unwrap();
        assert_eq!(captures.len(), 1);
        let HasMany::Loaded(illuminations) = &captures[0].illuminations else {
            panic!("illuminations should be loaded");
        };
        assert_eq!(illuminations.len(), 1);
        assert_eq!(illuminations[0].suggested_searches, None);
    }
}
