use sea_orm::prelude::*;

use crate::{
    common,
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
            created_at_human: common::humanize_datetime(capture_model.created_at),
            medias,
            illuminations,
        }
    }

    pub fn make_media_info(&self, media_model: model::media::ModelEx) -> MediaInfo {
        let handle = storage::StorageHandle::from(&media_model);

        MediaInfo {
            id: media_model.id,
            url: self.url_maker.make_url(&handle),

            mime_type: media_model.mime_type,
            hash_blake3: media_model.hash_blake3,

            storage_provider: media_model.storage_provider,
            storage_bucket: media_model.storage_bucket,
            storage_shard: media_model.storage_user_shard,
            storage_uuid: media_model.storage_uuid,
            storage_extension: media_model.storage_extension,
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

    pub fn make_spark_info(&self, spark_model: model::spark::ModelEx) -> SparkInfo {
        let mut input_pairs = match spark_model.spark_input_refs {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| (m.position, m.capture_id))
                .collect::<Vec<_>>(),
        };
        input_pairs.sort_by_key(|(position, _)| *position);
        let input_capture_ids = input_pairs
            .into_iter()
            .map(|(_, capture_id)| capture_id)
            .collect::<Vec<_>>();

        let spark_clusters = match spark_model.spark_clusters {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(|m| self.make_spark_cluster_info(m))
                .collect(),
        };

        let meta = match spark_model.spark_meta {
            HasOne::Unloaded => None,
            HasOne::NotFound => None,
            HasOne::Loaded(meta) => Some(SparkMetaInfo {
                provider_name: meta.provider_name,
                duration_ms: meta.duration_ms,
                input_capture_count: meta.input_capture_count,
                input_tokens: meta.input_tokens,
                output_tokens: meta.output_tokens,
                total_tokens: meta.total_tokens,
                provider_usage_json: meta.provider_usage_json,
                provider_grounding_json: meta.provider_grounding_json,
            }),
        };

        SparkInfo {
            id: spark_model.id,
            input_capture_ids,
            meta,
            spark_clusters,
        }
    }

    pub fn make_spark_cluster_info(
        &self,
        spark_cluster_model: model::spark_cluster::ModelEx,
    ) -> SparkClusterInfo {
        let spark_links = match spark_cluster_model.spark_links {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(SparkLinkInfo::from).collect(),
        };

        let referenced_capture_ids = match spark_cluster_model.spark_output_refs {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(|m| m.capture_id).collect(),
        };

        SparkClusterInfo {
            id: spark_cluster_model.id,
            title: spark_cluster_model.title,
            summary: spark_cluster_model.summary,
            referenced_capture_ids,
            spark_links,
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
