use chrono::{DateTime, Utc};
use sea_orm::prelude::*;
use serde::Serialize;

use crate::model;

use super::*;

#[derive(Clone, Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub created_at: DateTime<Utc>,
    pub medias: Vec<MediaInfo>,
    pub illuminations: Vec<IlluminationInfo>,
}

impl From<model::capture::ModelEx> for CaptureInfo {
    fn from(capture_model: model::capture::ModelEx) -> Self {
        let medias = match capture_model.medias {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(|m| MediaInfo::from(m)).collect(),
        };

        let illuminations = match capture_model.illuminations {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| IlluminationInfo::from(m))
                .collect(),
        };

        Self {
            id: capture_model.id,
            created_at: capture_model.created_at,
            medias,
            illuminations,
        }
    }
}
