use chrono::{DateTime, Utc};
use sea_orm::{EntityTrait, QueryOrder};
use serde::Serialize;

use crate::database::DbHandle;
use crate::model::*;

#[derive(Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub created_at: DateTime<Utc>,
    pub medias: Vec<media::Model>,
}

impl CaptureInfo {
    pub fn new(db_tuple: (capture::Model, Vec<media::Model>)) -> Self {
        Self {
            id: db_tuple.0.id,
            created_at: db_tuple.0.created_at,
            medias: db_tuple.1,
        }
    }

    pub async fn fetch_by_id(db: &DbHandle, capture_id: i32) -> anyhow::Result<CaptureInfo> {
        let fetch = capture::Entity::find_by_id(capture_id)
            .find_with_related(media::Entity)
            .all(&db.conn)
            .await
            .expect(&format!("Failed to fetch capture {} from db.", capture_id));

        match fetch.into_iter().next() {
            Some(db_tuple) => Ok(CaptureInfo::new(db_tuple)),
            None => Err(anyhow::anyhow!("Capture {} not found", capture_id)),
        }
    }

    pub async fn fetch_timeline(db: &DbHandle) -> anyhow::Result<Vec<CaptureInfo>> {
        let capture_infos = capture::Entity::find()
            .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
            .find_with_related(media::Entity)
            .all(&db.conn)
            .await
            .expect("Failed to fetch captures from db.")
            .into_iter()
            .map(|db_tuple| CaptureInfo::new(db_tuple))
            .collect::<Vec<_>>();

        Ok(capture_infos)
    }
}
