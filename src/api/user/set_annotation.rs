use anyhow::anyhow;
use chrono::Utc;
use sea_orm::prelude::*;
use sea_orm::{ActiveModelTrait, QueryOrder, Set};

use crate::{api::*, auth, database::DbHandle, model};

pub async fn set_annotation(
    db: &DbHandle,
    context: &auth::Context,
    capture_id: i32,
    content: String,
) -> Result<model::annotation::ModelEx, ApiError> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "annotation content must not be empty"
        )));
    }

    let capture = model::capture::Entity::find()
        .filter(model::capture::Column::Id.eq(capture_id))
        .filter(model::capture::Column::UserId.eq(context.user_id()))
        .one(&db.conn)
        .await?;

    if capture.is_none() {
        return Err(ApiError::not_found(anyhow!(
            "Capture with id {} not found or access denied",
            capture_id
        )));
    }

    let existing_active = model::annotation::Entity::find()
        .filter(model::annotation::Column::UserId.eq(context.user_id()))
        .filter(model::annotation::Column::CaptureId.eq(capture_id))
        .filter(model::annotation::Column::ArchivedAt.is_null())
        .order_by_desc(model::annotation::Column::Id)
        .one(&db.conn)
        .await?;

    let now = Utc::now();

    let annotation = if let Some(existing) = existing_active {
        if existing.content != content {
            let mut active: model::annotation::ActiveModel = existing.into();
            active.content = Set(content);
            active.updated_at = Set(now);
            active.update(&db.conn).await?
        } else {
            existing
        }
    } else {
        let active = model::annotation::ActiveModel {
            user_id: Set(context.user_id()),
            capture_id: Set(capture_id),
            content: Set(content),
            created_at: Set(now),
            updated_at: Set(now),
            archived_at: Set(None),
            ..Default::default()
        };
        active.insert(&db.conn).await?
    };

    let loaded = model::annotation::Entity::load()
        .filter(model::annotation::Column::Id.eq(annotation.id))
        .one(&db.conn)
        .await?;

    let Some(loaded) = loaded else {
        return Err(ApiError::internal(anyhow!(
            "failed to load annotation {} after write",
            annotation.id
        )));
    };

    Ok(loaded)
}