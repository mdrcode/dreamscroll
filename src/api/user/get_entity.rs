use anyhow::anyhow;
use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

/// Fetch one of the user's unarchived knowledge entities by occurrence ID.
pub async fn get_knowledge_entity(
    db: &DbHandle,
    context: &auth::Context,
    id: i32,
) -> Result<model::knowledge_entity::ModelEx, ApiError> {
    let entity = model::knowledge_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .filter(model::knowledge_entity::Column::UserId.eq(context.user_id()))
        .filter(model::knowledge_entity::Column::Id.eq(id))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .one(&db.conn)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow!("Knowledge entity with id {id} not found")))?;

    Ok(entity.into())
}

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
    use crate::test_support::{fixtures, test_db};

    /// One active and one archived capture, each with one entity named after
    /// its capture's state.
    async fn seed(db: &crate::database::DbHandle) -> (i32, Vec<i32>) {
        let user = fixtures::user(db, "entity_get").await;
        let active = fixtures::capture(db, user).await;
        let archived = fixtures::archived_capture(db, user).await;

        let mut ids = Vec::new();
        for (capture_id, name) in [(active, "active entity"), (archived, "archived entity")] {
            let illumination = fixtures::illumination(db, user, capture_id).await;
            ids.push(fixtures::entity(db, user, capture_id, illumination, 0, name).await);
        }

        (user, ids)
    }

    #[tokio::test]
    async fn entity_type_listing_excludes_archived_captures() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let (user, _) = seed(&db).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
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

    #[tokio::test]
    async fn entity_lookup_excludes_archived_captures() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let (user, entity_ids) = seed(&db).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let active = get_knowledge_entity(&db, &context, entity_ids[0])
            .await
            .unwrap();
        assert_eq!(active.name, "active entity");

        let archived = get_knowledge_entity(&db, &context, entity_ids[1])
            .await
            .unwrap_err();
        assert_eq!(archived.http_status_code, axum::http::StatusCode::NOT_FOUND);
    }
}
