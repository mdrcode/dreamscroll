//! Model builders for database tests.
//!
//! A user → capture → illumination → entity graph takes ~60 lines of
//! `ActiveModel::builder()` chains inline, and four test modules were each
//! spelling it out. These keep a test about behaviour instead of setup.

use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

use crate::{database::DbHandle, model};

/// One user's capture with its illumination — what observations hang off.
pub struct Capture {
    pub user_id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,
}

impl Capture {
    pub async fn new(db: &DbHandle, tag: &str) -> Self {
        let user_id = user(db, tag).await;
        let capture_id = capture(db, user_id).await;
        let illumination_id = illumination(db, user_id, capture_id).await;
        Self {
            user_id,
            capture_id,
            illumination_id,
        }
    }
}

/// One thing a reading claimed to see, in a capture.
#[allow(clippy::too_many_arguments)]
pub async fn observe(
    db: &DbHandle,
    capture: &Capture,
    position: i32,
    name: &str,
    description: Option<&str>,
    entity_type: &str,
    platform: Option<&str>,
    handle: Option<&str>,
) -> i32 {
    model::user_observed_entity::Entity::insert(model::user_observed_entity::ActiveModel {
        user_id: Set(capture.user_id),
        capture_id: Set(capture.capture_id),
        illumination_id: Set(capture.illumination_id),
        position: Set(position),
        name: Set(name.to_string()),
        entity_type: Set(entity_type.to_string()),
        description: Set(description.map(str::to_string)),
        platform: Set(platform.map(str::to_string)),
        platform_handle: Set(handle.map(str::to_string)),
        ..Default::default()
    })
    .exec(&db.conn)
    .await
    .expect("observation should insert")
    .last_insert_id as i32
}

/// An observation that already belongs to an entity — the state a merge starts from.
pub async fn attached(
    db: &DbHandle,
    capture: &Capture,
    position: i32,
    name: &str,
    description: Option<&str>,
    entity_type: &str,
    knowledge_entity_id: i32,
) -> i32 {
    model::user_observed_entity::Entity::insert(model::user_observed_entity::ActiveModel {
        user_id: Set(capture.user_id),
        capture_id: Set(capture.capture_id),
        illumination_id: Set(capture.illumination_id),
        position: Set(position),
        name: Set(name.to_string()),
        entity_type: Set(entity_type.to_string()),
        description: Set(description.map(str::to_string)),
        knowledge_entity_id: Set(Some(knowledge_entity_id)),
        ..Default::default()
    })
    .exec(&db.conn)
    .await
    .expect("observation should insert")
    .last_insert_id as i32
}

/// How many entities exist, for asserting how a reading was clustered.
pub async fn entity_count(db: &DbHandle) -> u64 {
    model::knowledge_entity::Entity::find()
        .count(&db.conn)
        .await
        .unwrap()
}

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
/// One knowledge entity with one observation attached, created fresh.
///
/// The shape projection and discard both operate on: a row whose columns are derived from its
/// members, and one member to derive them from.
/// A claim, the shape the resolver or a capture writes: the identity, then the edge to it.
pub async fn claim(
    db: &DbHandle,
    knowledge_entity_id: i32,
    scheme: &str,
    external_id: &str,
    provider: &str,
    method: Option<&str>,
    confidence: Option<f32>,
) {
    let identity = crate::knowledge::evidence::Evidence::from_handle(db)
        .upsert_identity(&crate::knowledge::evidence::IdentityDraft {
            scheme: scheme.to_string(),
            external_id: external_id.to_string(),
            provider: provider.to_string(),
            method: method.map(str::to_string),
            confidence,
            label: None,
            description: None,
            classes: Vec::new(),
        })
        .await
        .unwrap();
    model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
        knowledge_entity_id: sea_orm::Set(knowledge_entity_id),
        knowledge_identity_id: sea_orm::Set(identity),
        provider: sea_orm::Set(provider.to_string()),
        method: sea_orm::Set(method.map(str::to_string)),
        ..Default::default()
    })
    .exec(&db.conn)
    .await
    .unwrap();
}

pub async fn knowledge_entity_with_member(db: &DbHandle, name: &str) -> i32 {
    let user_id = user(db, "project").await;
    let capture_id = capture(db, user_id).await;
    let illumination_id = illumination(db, user_id, capture_id).await;
    let observation_id = entity(db, user_id, capture_id, illumination_id, 0, name).await;

    let knowledge_entity_id =
        model::knowledge_entity::Entity::insert(model::knowledge_entity::ActiveModel {
            name: sea_orm::Set(name.to_string()),
            entity_type: sea_orm::Set("real_person".to_string()),
            status: sea_orm::Set(crate::model::knowledge_entity::Status::Provisional),
            origin_observation_id: sea_orm::Set(Some(observation_id)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;

    model::user_observed_entity::Entity::update_many()
        .col_expr(
            model::user_observed_entity::Column::KnowledgeEntityId,
            sea_orm::sea_query::Expr::value(Some(knowledge_entity_id)),
        )
        .filter(model::user_observed_entity::Column::Id.eq(observation_id))
        .exec(&db.conn)
        .await
        .unwrap();

    knowledge_entity_id
}

/// The durable key a merge is logged under: the observation the entity was created from.
pub async fn origin(db: &DbHandle, knowledge_entity_id: i32) -> i32 {
    model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
        .origin_observation_id
        .expect("the fixture set an origin")
}

/// Rewrites an entity's origin, standing in for what `link` would have chosen — the way a test
/// puts a *rebuilt* entity back under an origin a log row was written for.
pub async fn set_origin(db: &DbHandle, knowledge_entity_id: i32, origin: i32) {
    model::knowledge_entity::Entity::update_many()
        .col_expr(
            model::knowledge_entity::Column::OriginObservationId,
            sea_orm::sea_query::Expr::value(Some(origin)),
        )
        .filter(model::knowledge_entity::Column::Id.eq(knowledge_entity_id))
        .exec(&db.conn)
        .await
        .unwrap();
}

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
        status: crate::model::knowledge_entity::Status::Provisional,
        confidence: None,
        merged_into_id: None,
        origin_observation_id: None,
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

/// The same, for one illumination. Its `observations` start unloaded; a caller that needs them sets
/// the relation explicitly, which is also what the linker refuses to run without.
pub fn stub_illumination(id: i32) -> model::illumination::ModelEx {
    let model = model::illumination::Model {
        id,
        user_id: 1,
        capture_id: 1,
        task_run_id: format!("task-{id}"),
        inference_meta_id: None,
        created_at: Utc::now(),
        summary: "a summary".to_string(),
        details: "the details".to_string(),
        suggested_searches: None,
    };
    model.into()
}
