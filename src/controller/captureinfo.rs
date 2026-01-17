use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sea_orm::prelude::*;
use sea_orm::{EntityLoaderTrait, EntityTrait, QueryOrder, QuerySelect};
use serde::Serialize;

use crate::common::*;
use crate::database::DbHandle;
use crate::entity::*;

#[derive(Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub created_at: DateTime<Utc>,
    pub medias: Vec<media::ModelEx>,
    pub illuminations: Vec<illumination::ModelEx>,

    pub summary: String,
    pub details: String,
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

        // TODO obviously this is brittle, need to rethink the CaptureInfo
        // structure and how elements are extracted from the illumination
        let mut summary = String::new();
        let mut details = String::new();

        if illuminations.len() > 0 {
            let first_illum = &illuminations[0];
            let parts: Vec<&str> = first_illum.content.splitn(2, "\n\n").collect();
            if parts.len() > 0 {
                summary = parts[0].to_string();
            }
            if parts.len() > 1 {
                details = parts[1].to_string();
            }
        }

        Self {
            id: mx.id,
            created_at: mx.created_at,
            medias,
            illuminations,
            summary: summary,
            details: details,
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
            .all(&db.conn)
            .await?;

        let illuminations = captures
            .load_many(
                illumination::Entity::find()
                    //.filter(illumination::Column::Provider.eq("gemini"))
                    .order_by(illumination::Column::Id, sea_orm::Order::Desc),
                &db.conn,
            )
            .await?;

        let capture_infos = captures
            .into_iter()
            .zip(illuminations.into_iter())
            .map(|(c, ill)| {
                let mut mx = c;
                mx.illuminations =
                    HasMany::Loaded(ill.into_iter().map(illumination::ModelEx::from).collect());
                CaptureInfo::new(mx)
            })
            .collect();

        Ok(capture_infos)
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

    pub async fn search_by_illuminations(
        db: &DbHandle,
        query: &str,
    ) -> anyhow::Result<Vec<CaptureInfo>, AppError> {
        if query.is_empty() {
            return Ok(vec![]);
        }

        // Find illuminations that contain the search query
        let capture_ids_with_match = illumination::Entity::find()
            .filter(illumination::Column::Content.contains(query))
            .column(illumination::Column::CaptureId)
            .distinct()
            .all(&db.conn)
            .await?
            .into_iter()
            .map(|i| i.capture_id)
            .collect::<Vec<i32>>();

        // Get unique capture IDs

        let captures = capture::Entity::load()
            .filter(capture::Column::Id.is_in(capture_ids_with_match))
            .with(media::Entity)
            .with(illumination::Entity)
            .all(&db.conn)
            .await?;

        let capture_info = captures.into_iter().map(|c| CaptureInfo::new(c)).collect();

        Ok(capture_info)
    }
}
