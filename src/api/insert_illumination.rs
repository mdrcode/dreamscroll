use sea_orm::EntityTrait;

use crate::{api, auth, database::DbHandle, illumination::*, model};

pub async fn insert_illumination(
    db: &DbHandle,
    _context: &auth::Context,
    capture: &api::CaptureInfo,
    illumination: Illumination,
) -> Result<(), api::ApiError> {
    // Todo, the illumination::Illumination struct has diverged from the
    // model::illumination::ActiveModel, need to think about naming carefully
    // possibly we will change the name of model::illumination to model::exposition
    model::illumination::ActiveModel::builder()
        .set_capture_id(capture.id)
        .set_summary(&illumination.summary)
        .set_details(&illumination.details)
        .save(&db.conn)
        .await?;

    let knode_builders = illumination.entities.iter().map(|entity| {
        model::knode::ActiveModel::builder()
            .set_capture_id(capture.id)
            .set_name(&entity.name)
            .set_description(&entity.description)
            .set_k_type(entity.entity_type.to_string())
    });
    model::knode::Entity::insert_many(knode_builders)
        .exec(&db.conn)
        .await?;

    let xquery_builders = illumination.suggested_searches.iter().map(|ss| {
        model::xquery::ActiveModel::builder()
            .set_capture_id(capture.id)
            .set_query(ss)
    });
    model::xquery::Entity::insert_many(xquery_builders)
        .exec(&db.conn)
        .await?;

    let social_media_builders = illumination.social_media_accounts.iter().map(|sm| {
        model::social_media::ActiveModel::builder()
            .set_capture_id(capture.id)
            .set_display_name(&sm.display_name)
            .set_handle(&sm.handle)
            .set_platform(sm.platform.to_string())
    });
    model::social_media::Entity::insert_many(social_media_builders)
        .exec(&db.conn)
        .await?;

    // An insult to search indexes everywhere, but it'll do for now
    let raw_content_for_search = format!(
        "{} {} {} {} {}",
        illumination.summary,
        illumination.details,
        illumination
            .entities
            .into_iter()
            .map(|e| e.name)
            .collect::<Vec<String>>()
            .join(" "),
        illumination
            .suggested_searches
            .into_iter()
            .collect::<Vec<String>>()
            .join(" "),
        illumination
            .social_media_accounts
            .into_iter()
            .map(|s| s.display_name)
            .collect::<Vec<String>>()
            .join(" ")
    );

    model::search_index::ActiveModel::builder()
        // Note that user_id comes from the passed CaptureInfo, *not* the auth context
        // for the case when backend service is illuminating on behalf of a user
        .set_user_id(capture.user_id)
        .set_capture_id(capture.id)
        .set_content(raw_content_for_search)
        .save(&db.conn)
        .await?;

    Ok(())
}
