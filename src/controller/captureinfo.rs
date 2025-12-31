use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sea_orm::prelude::*;
use sea_orm::{EntityLoaderTrait, EntityTrait, QueryOrder, QuerySelect};
use serde::Serialize;

use crate::common::*;
use crate::database::DbHandle;
use crate::model::*;

#[derive(Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub created_at: DateTime<Utc>,
    pub medias: Vec<media::ModelEx>,
    pub illuminations: Vec<illumination::ModelEx>,
}

impl CaptureInfo {
    pub fn new(mx: capture::ModelEx) -> Self {
        // TODO is this the most idiomatic way??
        let medias = match mx.medias {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(medias) => medias,
        };

        let illuminations = match mx.illuminations {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(illuminations) => illuminations,
        };

        Self {
            id: mx.id,
            created_at: mx.created_at,
            medias,
            illuminations,
        }
    }

    pub async fn fetch_by_id(db: &DbHandle, id: i32) -> anyhow::Result<CaptureInfo, AppError> {
        let capture = capture::Entity::load()
            .filter_by_id(id)
            .with(media::Entity)
            .with(illumination::Entity)
            .one(&db.conn)
            .await
            .map_err(|e| {
                AppError::internal(anyhow!("DB error fetching capture id {}: {}", id, e))
            })?;

        match capture {
            Some(capture) => Ok(CaptureInfo::new(capture)),
            None => Err(AppError::not_found(anyhow!("Capture id {} not found", id))),
        }
    }

    // TODO obviously this should take a user_id or equivalent at some point
    pub async fn fetch_timeline(db: &DbHandle) -> anyhow::Result<Vec<CaptureInfo>, AppError> {
        let captures = capture::Entity::load()
            .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
            .with(media::Entity)
            .with(illumination::Entity)
            .all(&db.conn)
            .await
            .map_err(|e| AppError::internal(anyhow!("Failed to fetch captures from db: {}", e)))?
            .into_iter()
            .map(|c| CaptureInfo::new(c))
            .collect::<Vec<_>>();

        Ok(captures)
    }

    pub async fn fetch_ids_need_illumination(db: &DbHandle) -> anyhow::Result<Vec<i32>, AppError> {
        let capture_ids = capture::Entity::find()
            .left_join(illumination::Entity)
            .filter(illumination::Column::Id.is_null())
            .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
            .select_only()
            .column(capture::Column::Id)
            .into_tuple::<i32>()
            .all(&db.conn)
            .await?;

        Ok(capture_ids)
    }
}
