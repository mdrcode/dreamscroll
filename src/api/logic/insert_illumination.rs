use sea_orm::EntityTrait;

use crate::{api, auth, database::DbHandle, illumination::*, model};

// Is this still needed?
pub async fn insert_illumination(
    db: &DbHandle,
    _user_context: &auth::Context,
    capture_id: i32,
    illumination: Illumination,
) -> Result<(), api::ApiError> {
    let raw_content = format!("{}\n{}", illumination.summary, illumination.details);

    model::illumination::ActiveModel::builder()
        .set_capture_id(capture_id)
        .set_summary(illumination.summary)
        .set_details(illumination.details)
        .save(&db.conn)
        .await?;

    let knode_builders = illumination.entities.into_iter().map(|entity| {
        model::knode::ActiveModel::builder()
            .set_capture_id(capture_id)
            .set_name(entity.name)
            .set_description(entity.description)
            .set_k_type(entity.entity_type.to_string())
    });
    model::knode::Entity::insert_many(knode_builders)
        .exec(&db.conn)
        .await?;

    let xquery_builders = illumination.suggested_searches.into_iter().map(|ss| {
        model::xquery::ActiveModel::builder()
            .set_capture_id(capture_id)
            .set_query(ss)
    });
    model::xquery::Entity::insert_many(xquery_builders)
        .exec(&db.conn)
        .await?;

    let social_media_builders = illumination.social_media_accounts.into_iter().map(|sm| {
        model::social_media::ActiveModel::builder()
            .set_capture_id(capture_id)
            .set_display_name(sm.display_name)
            .set_handle(sm.handle)
            .set_platform(sm.platform.to_string())
    });
    model::social_media::Entity::insert_many(social_media_builders)
        .exec(&db.conn)
        .await?;

    Ok(())
}
