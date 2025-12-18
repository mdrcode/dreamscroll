use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{db::DbHandle, entity::capture};

#[derive(Serialize)]
pub struct ImageInfo {
    pub filename: String,
    pub created_at: String,
}

pub async fn collect_images_db(db: &DbHandle) -> Vec<ImageInfo> {
    use crate::entity::media;
    use sea_orm::EntityTrait;

    let mut images = Vec::new();

    let conn = &db.conn;

    if let Ok(media_records) = media::Entity::find().all(conn).await {
        for record in media_records {
            if let Some(capture) = record.capture_id {
                if let Ok(capture_record) = capture::Entity::find_by_id(capture).one(conn).await {
                    if let Some(capture) = capture_record {
                        let created_at: DateTime<Utc> = capture.created_at.into();
                        let created_fmt = created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
                        images.push(ImageInfo {
                            filename: record.filename,
                            created_at: created_fmt,
                        });
                    }
                }
            }
        }
    }

    images
}


