use sea_orm::prelude::*;

use crate::{
    model::{self},
    storage,
};

use super::*;

#[derive(Clone)]
pub struct InfoMaker {
    url_maker: storage::UrlMaker,
}

impl InfoMaker {
    pub fn new(url_maker: storage::UrlMaker) -> Self {
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
                .map(|m| self.make_illumination_info(m))
                .collect(),
        };

        CaptureInfo {
            id: capture_model.id,
            user_id: capture_model.user_id,
            created_at: capture_model.created_at,
            medias,
            illuminations,
        }
    }

    pub fn make_media_info(&self, media_model: model::media::ModelEx) -> MediaInfo {
        let storage_id = storage::StorageIdentity::from(&media_model);

        MediaInfo {
            id: media_model.id,
            url: self.url_maker.make_url(&storage_id),

            storage_provider: media_model.storage_provider,
            storage_bucket: media_model.storage_bucket,
            storage_shard: media_model.storage_shard,
            storage_id: media_model.storage_id,
        }
    }

    pub fn make_illumination_info(
        &self,
        illumination_model: model::illumination::ModelEx,
    ) -> IlluminationInfo {
        let x_queries = match illumination_model.xqueries {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(|m| m.query).collect(),
        };

        let k_nodes = match illumination_model.knodes {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(|m| KNodeInfo::from(m)).collect(),
        };

        let social_medias = match illumination_model.social_medias {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| SocialMediaInfo::from(m))
                .collect(),
        };

        IlluminationInfo {
            id: illumination_model.id,
            capture_id: illumination_model.capture_id,
            summary: illumination_model.summary,
            details: illumination_model.details,
            x_queries,
            k_nodes,
            social_medias,
        }
    }

    pub fn make_knode_entity_info(
        &self,
        knode_model: model::knode::ModelEx,
        capture: model::capture::ModelEx,
    ) -> EntityInfo {
        let capture_info = self.make_capture_info(capture);

        EntityInfo::KNode {
            id: knode_model.id,
            name: knode_model.name,
            description: knode_model.description,
            k_type: knode_model.k_type,
            capture: capture_info,
        }
    }

    pub fn make_social_media_entity_info(
        &self,
        social_media_model: model::social_media::ModelEx,
        capture: model::capture::ModelEx,
    ) -> EntityInfo {
        let capture_info = self.make_capture_info(capture);

        EntityInfo::SocialMedia {
            id: social_media_model.id,
            display_name: social_media_model.display_name,
            handle: social_media_model.handle,
            platform: social_media_model.platform,
            capture: capture_info,
        }
    }
}
