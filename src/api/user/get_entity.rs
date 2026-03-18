use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

/// Fetch a KNode entity by ID along with its associated capture
pub async fn get_knode(
    db: &DbHandle,
    context: &auth::Context,
    knode_id: i32,
) -> Result<(model::knode::ModelEx, model::capture::ModelEx), ApiError> {
    let knode = model::knode::Entity::find_by_id(knode_id)
        .one(&db.conn)
        .await?;

    let Some(knode) = knode else {
        return Err(ApiError::not_found(anyhow!(
            "KNode with id {} not found",
            knode_id
        )));
    };

    let capture = super::get_captures(&db, context, vec![knode.capture_id]).await?;

    let Some(capture) = capture.into_iter().next() else {
        return Err(ApiError::not_found(anyhow!(
            "KNode with id {} not found or access denied",
            knode_id
        )));
    };

    Ok((knode.into(), capture.into()))
}

/// Fetch a SocialMedia entity by ID along with its associated capture
pub async fn get_social_media(
    db: &DbHandle,
    context: &auth::Context,
    social_media_id: i32,
) -> Result<(model::social_media::ModelEx, model::capture::ModelEx), ApiError> {
    let social_media = model::social_media::Entity::find_by_id(social_media_id)
        .one(&db.conn)
        .await?;

    let Some(social_media) = social_media else {
        return Err(ApiError::not_found(anyhow!(
            "SocialMedia with id {} not found",
            social_media_id
        )));
    };

    let capture = super::get_captures(&db, context, vec![social_media.capture_id]).await?;

    let Some(capture) = capture.into_iter().next() else {
        return Err(ApiError::not_found(anyhow!(
            "SocialMedia with id {} not found or access denied",
            social_media_id
        )));
    };

    Ok((social_media.into(), capture.into()))
}
