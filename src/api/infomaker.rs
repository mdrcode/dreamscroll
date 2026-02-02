use sea_orm::prelude::*;

use crate::{model, storage};

use super::*;

#[derive(Clone)]
pub struct InfoMaker {
    url_maker: storage::StorageUrlMaker,
}

impl InfoMaker {
    pub fn new(url_maker: storage::StorageUrlMaker) -> Self {
        Self { url_maker }
    }

    pub fn make_capture_info(&self, capture_model: model::capture::ModelEx) -> CaptureInfo {
        let medias = match capture_model.medias {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| self.make_media_info(m))
                .collect(),
        };

        let illuminations = match capture_model.illuminations {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| IlluminationInfo::from(m))
                .collect(),
        };

        let x_queries = match capture_model.xqueries {
            sea_orm::prelude::HasMany::Unloaded => vec![],
            sea_orm::prelude::HasMany::Loaded(models) => {
                models.into_iter().map(|m| m.query).collect()
            }
        };

        let k_nodes = match capture_model.knodes {
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

        CaptureInfo {
            id: capture_model.id,
            user_id: capture_model.user_id,
            created_at: capture_model.created_at,
            medias,
            illuminations,
            x_queries,
            k_nodes,
            social_medias,
        }
    }

    pub fn make_media_info(&self, media_model: model::media::ModelEx) -> MediaInfo {
        let storage_id = storage::StorageIdentity::from(&media_model);

        MediaInfo {
            id: media_model.id,
            url: self.url_maker.make_url(&storage_id),
            storage_id: storage_id.provider_id,
        }
    }
}
