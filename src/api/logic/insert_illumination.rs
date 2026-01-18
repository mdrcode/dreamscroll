use crate::{common::*, database::DbHandle, entity, illumination::Illuminator};

// Is this still needed?
pub async fn insert_illumination<I: Illuminator>(
    db: &DbHandle,
    capture_id: i32,
    illuminator: &I,
    content: String,
) -> anyhow::Result<(), AppError> {
    entity::illumination::ActiveModel::builder()
        .set_capture_id(capture_id)
        .set_provider(illuminator.model_name().to_string())
        .set_content(content)
        .save(&db.conn)
        .await?;

    Ok(())
}
