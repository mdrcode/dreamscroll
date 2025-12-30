use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};
use serde::Serialize;

use crate::common::*;
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

    pub async fn fetch_by_id(db: &DbHandle, id: i32) -> anyhow::Result<CaptureInfo, AppError> {
        let fetch = capture::Entity::find_by_id(id)
            .find_with_related(media::Entity)
            .all(&db.conn)
            .await
            .map_err(|e| {
                AppError::internal(anyhow!("DB error fetching capture id {}: {}", id, e))
            })?;

        match fetch.into_iter().next() {
            Some(db_tuple) => Ok(CaptureInfo::new(db_tuple)),
            None => Err(AppError::not_found(anyhow!("Capture id {} not found", id))),
        }
    }

    // TODO obviously this should take a user_id or equivalent at some point
    pub async fn fetch_timeline(db: &DbHandle) -> anyhow::Result<Vec<CaptureInfo>, AppError> {
        let capture_infos = capture::Entity::find()
            .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
            .find_with_related(media::Entity)
            .all(&db.conn)
            .await
            .map_err(|e| AppError::internal(anyhow!("Failed to fetch captures from db: {}", e)))?
            .into_iter()
            .map(|db_tuple| CaptureInfo::new(db_tuple))
            .collect::<Vec<_>>();

        Ok(capture_infos)
    }

    pub async fn fetch_ids_need_illumination(db: &DbHandle) -> anyhow::Result<Vec<i32>, AppError> {
        let capture_ids = capture::Entity::find()
            .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
            .select_only()
            .column(capture::Column::Id)
            .into_tuple::<i32>()
            .all(&db.conn)
            .await?;

        Ok(capture_ids)
    }
}
