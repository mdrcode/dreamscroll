use chrono::{DateTime, Utc};
use sea_orm::prelude::*;
use serde::Serialize;

use crate::model;

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub created_at: DateTime<Utc>,
    pub medias: Vec<MediaInfo>,
    pub illuminations: Vec<IlluminationInfo>,
    pub x_queries: Vec<String>,
    pub k_nodes: Vec<KNodeInfo>,
    pub social_medias: Vec<SocialMediaInfo>,
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

        let x_queries = match capture_model.x_queries {
            sea_orm::prelude::HasMany::Unloaded => vec![],
            sea_orm::prelude::HasMany::Loaded(models) => {
                models.into_iter().map(|m| m.query).collect()
            }
        };

        let k_nodes = match capture_model.k_nodes {
            sea_orm::prelude::HasMany::Unloaded => vec![],
            sea_orm::prelude::HasMany::Loaded(models) => {
                models.into_iter().map(|m| KNodeInfo::from(m)).collect()
            }
        };

        let social_medias = match capture_model.social_medias {
            sea_orm::prelude::HasMany::Unloaded => vec![],
            sea_orm::prelude::HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| SocialMediaInfo::from(m))
                .collect(),
        };

        Self {
            id: capture_model.id,
            created_at: capture_model.created_at,
            medias,
            illuminations,
            x_queries,
            k_nodes,
            social_medias,
        }
    }
}
