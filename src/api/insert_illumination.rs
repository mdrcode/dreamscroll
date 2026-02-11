use crate::{api, auth, database::DbHandle, illumination::*, model};

pub async fn insert_illumination(
    db: &DbHandle,
    _context: &auth::Context,
    capture: &api::CaptureInfo,
    illumination: Illumination,
) -> Result<(), api::ApiError> {
    let mut builder = model::illumination::ActiveModel::builder()
        .set_capture_id(capture.id)
        .set_summary(&illumination.summary)
        .set_details(&illumination.details)
        .set_search_index(
            model::search_index::ActiveModel::builder()
                .set_user_id(capture.user_id)
                .set_content(format_for_search(&illumination)),
        );

    for entity in &illumination.entities {
        builder.knodes.push(
            model::knode::ActiveModel::builder()
                .set_name(&entity.name)
                .set_description(&entity.description)
                .set_k_type(entity.entity_type.to_string()),
        );
    }

    for xquery in &illumination.suggested_searches {
        builder
            .xqueries
            .push(model::xquery::ActiveModel::builder().set_query(xquery));
    }

    for sm in &illumination.social_media_accounts {
        builder.social_medias.push(
            model::social_media::ActiveModel::builder()
                .set_display_name(&sm.display_name)
                .set_handle(&sm.handle)
                .set_platform(sm.platform.to_string()),
        );
    }

    builder.save(&db.conn).await?;

    Ok(())
}

pub fn format_for_search(illumination: &Illumination) -> String {
    // lol, a naive approach for now
    format!(
        "{} {} {} {} {}",
        illumination.summary,
        illumination.details,
        illumination
            .entities
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<String>>()
            .join(" "),
        illumination
            .suggested_searches
            .iter()
            .cloned()
            .collect::<Vec<String>>()
            .join(" "),
        illumination
            .social_media_accounts
            .iter()
            .map(|s| s.display_name.clone())
            .collect::<Vec<String>>()
            .join(" ")
    )
}
