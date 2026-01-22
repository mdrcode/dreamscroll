use crate::{api, database::DbHandle, illumination::Illuminator, model};

// Is this still needed?
pub async fn insert_illumination<I: Illuminator>(
    db: &DbHandle,
    capture_id: i32,
    illuminator: &I,
    content: String,
) -> Result<(), api::ApiError> {
    model::illumination::ActiveModel::builder()
        .set_capture_id(capture_id)
        .set_provider(illuminator.model_name().to_string())
        .set_content(content)
        .save(&db.conn)
        .await?;

    Ok(())
}
