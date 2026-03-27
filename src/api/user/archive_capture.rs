use anyhow::anyhow;
use chrono::Utc;
use sea_orm::prelude::*;
use sea_orm::{ActiveModelTrait, Set};

use crate::{api::*, auth, database::DbHandle, model};

async fn set_capture_archive_state(
    db: &DbHandle,
    context: &auth::Context,
    capture_id: i32,
    archived: bool,
) -> Result<(), ApiError> {
    let capture = model::capture::Entity::find()
        .filter(model::capture::Column::Id.eq(capture_id))
        .filter(model::capture::Column::UserId.eq(context.user_id()))
        .one(&db.conn)
        .await?;

    let Some(capture) = capture else {
        return Err(ApiError::not_found(anyhow!(
            "Capture with id {} not found or access denied",
            capture_id
        )));
    };

    let mut active: model::capture::ActiveModel = capture.into();
    active.archived_at = Set(if archived { Some(Utc::now()) } else { None });
    active.update(&db.conn).await?;

    Ok(())
}

pub async fn archive_capture(
    db: &DbHandle,
    context: &auth::Context,
    capture_id: i32,
) -> Result<(), ApiError> {
    set_capture_archive_state(db, context, capture_id, true).await
}

pub async fn unarchive_capture(
    db: &DbHandle,
    context: &auth::Context,
    capture_id: i32,
) -> Result<(), ApiError> {
    set_capture_archive_state(db, context, capture_id, false).await
}
