use anyhow::anyhow;
use chrono::Utc;
use sea_orm::prelude::*;
use sea_orm::{ActiveModelTrait, QueryOrder, Set};

use crate::{api::*, auth, database::DbHandle, model};

pub async fn archive_annotation(
    db: &DbHandle,
    context: &auth::Context,
    capture_id: i32,
) -> Result<(), ApiError> {
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

    let Some(annotation) = existing_active else {
        return Ok(());
    };

    let now = Utc::now();
    let mut active: model::annotation::ActiveModel = annotation.into();
    active.archived_at = Set(Some(now));

    active.update(&db.conn).await?;

    Ok(())
}