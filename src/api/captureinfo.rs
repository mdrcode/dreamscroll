use chrono::{DateTime, Utc};
use sea_orm::prelude::*;
use serde::Serialize;

use crate::entity::*;

#[derive(Clone, Serialize)]
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
}
