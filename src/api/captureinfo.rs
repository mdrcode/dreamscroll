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

    pub summary: String,
    pub details: String,
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
            id: capture_model.id,
            created_at: capture_model.created_at,
            medias,
            illuminations,
            summary,
            details,
        }
    }
}
