//! Model builders for database tests.
//!
//! A user → capture → illumination → entity graph takes ~60 lines of
//! `ActiveModel::builder()` chains inline, and four test modules were each
//! spelling it out. These keep a test about behaviour instead of setup.

use chrono::Utc;

use crate::{database::DbHandle, model};

pub async fn user(db: &DbHandle, tag: &str) -> i32 {
    model::user::ActiveModel::builder()
        .set_username(format!("{tag}_{}", uuid::Uuid::new_v4().simple()))
        .set_password_hash("test-password-hash")
        .set_storage_shard(model::user::generate_storage_shard(8))
        .insert(&db.conn)
        .await
        .unwrap()
        .id
}

pub async fn capture(db: &DbHandle, user_id: i32) -> i32 {
    model::capture::ActiveModel::builder()
        .set_user_id(user_id)
        .set_created_at(Utc::now())
        .insert(&db.conn)
        .await
        .unwrap()
        .id
}

pub async fn archived_capture(db: &DbHandle, user_id: i32) -> i32 {
    model::capture::ActiveModel::builder()
        .set_user_id(user_id)
        .set_created_at(Utc::now())
        .set_archived_at(Some(Utc::now()))
        .insert(&db.conn)
        .await
        .unwrap()
        .id
}

pub async fn illumination(db: &DbHandle, user_id: i32, capture_id: i32) -> i32 {
    model::illumination::ActiveModel::builder()
        .set_user_id(user_id)
        .set_capture_id(capture_id)
        .set_task_run_id(uuid::Uuid::new_v4().to_string())
        .set_summary("summary")
        .set_details("")
        .set_suggested_searches(Some(vec![]))
        .insert(&db.conn)
        .await
        .unwrap()
        .id
}

/// `position` is part of the `(illumination_id, position)` unique key, so two
/// entities in one illumination must not share it.
pub async fn entity(
    db: &DbHandle,
    user_id: i32,
    capture_id: i32,
    illumination_id: i32,
    position: i32,
    name: &str,
) -> i32 {
    model::user_observed_entity::ActiveModel::builder()
        .set_user_id(user_id)
        .set_capture_id(capture_id)
        .set_illumination_id(illumination_id)
        .set_position(position)
        .set_name(name)
        .set_entity_type("unknown")
        .set_description(None)
        .set_search_query(None)
        .set_platform(None)
        .set_platform_handle(None)
        .set_platform_display_name(None)
        .insert(&db.conn)
        .await
        .unwrap()
        .id
}

/// An in-memory knowledge entity: the fields a strategy reads, and nothing else.
///
/// The async builders above insert a graph; this one is for the tests that only need something to
/// hand a `Blocker`, `Linker` or `Resolver`. Never inserted, so changing it cannot affect a query.
pub fn stub_entity(id: i32, name: &str, entity_type: &str) -> model::knowledge_entity::Model {
    let now = Utc::now();
    model::knowledge_entity::Model {
        id,
        created_at: now,
        updated_at: now,
        name: name.to_string(),
        alt_names: Vec::new(),
        entity_type: entity_type.to_string(),
        alt_types: Vec::new(),
        description: None,
        alt_descriptions: Vec::new(),
        search_query: None,
        alt_search_queries: Vec::new(),
        platform: None,
        platform_handle: None,
        platform_display_name: None,
        status: "provisional".to_string(),
        confidence: None,
        merged_into_id: None,
    }
}

/// The same, for one observation.
pub fn stub_observation(
    id: i32,
    name: &str,
    entity_type: &str,
) -> model::user_observed_entity::Model {
    model::user_observed_entity::Model {
        id,
        user_id: 1,
        capture_id: 1,
        illumination_id: 1,
        position: 0,
        created_at: Utc::now(),
        name: name.to_string(),
        entity_type: entity_type.to_string(),
        description: None,
        search_query: None,
        platform: None,
        platform_handle: None,
        platform_display_name: None,
        knowledge_entity_id: None,
    }
}

