use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

/// Fetch the user's unarchived knowledge entities of the requested type.
pub async fn get_knowledge_entities_by_type(
    db: &DbHandle,
    context: &auth::Context,
    entity_type: &str,
) -> Result<Vec<model::knowledge_entity::Model>, ApiError> {
    Ok(model::knowledge_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .filter(model::knowledge_entity::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(model::knowledge_entity::Column::EntityType.eq(entity_type))
        .order_by_asc(model::knowledge_entity::Column::Id)
        .all(&db.conn)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn entity_type_listing_excludes_archived_captures() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = model::user::ActiveModel::builder()
            .set_username(format!("entity_list_{}", uuid::Uuid::new_v4().simple()))
            .set_password_hash("test-password-hash")
            .set_storage_shard(model::user::generate_storage_shard(8))
            .insert(&db.conn)
            .await
            .unwrap();

        let active_capture = model::capture::ActiveModel::builder()
            .set_user_id(user.id)
            .set_created_at(Utc::now())
            .insert(&db.conn)
            .await
            .unwrap();
        let archived_capture = model::capture::ActiveModel::builder()
            .set_user_id(user.id)
            .set_created_at(Utc::now())
            .set_archived_at(Some(Utc::now()))
            .insert(&db.conn)
            .await
            .unwrap();

        for (capture_id, name) in [
            (active_capture.id, "active entity"),
            (archived_capture.id, "archived entity"),
        ] {
            let illumination = model::illumination::ActiveModel::builder()
                .set_user_id(user.id)
                .set_capture_id(capture_id)
                .set_summary(name)
                .set_details("")
                .set_suggested_searches(Some(vec![]))
                .insert(&db.conn)
                .await
                .unwrap();
            model::knowledge_entity::ActiveModel::builder()
                .set_user_id(user.id)
                .set_capture_id(capture_id)
                .set_illumination_id(illumination.id)
                .set_position(0)
                .set_name(name)
                .set_entity_type("unknown")
                .set_description(None)
                .set_search_query(None)
                .set_platform(None)
                .set_platform_handle(None)
                .set_platform_display_name(None)
                .insert(&db.conn)
                .await
                .unwrap();
        }

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user.id));
        let entities = get_knowledge_entities_by_type(&db, &context, "unknown")
            .await
            .unwrap();
        assert_eq!(
            entities
                .into_iter()
                .map(|entity| entity.name)
                .collect::<Vec<_>>(),
            vec!["active entity"]
        );
    }
}
