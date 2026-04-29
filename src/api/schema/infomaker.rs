use std::collections::HashMap;

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

        let annotation = match capture_model.annotations {
            HasMany::Unloaded => None,
            HasMany::Loaded(models) => models
                .into_iter()
                .filter(|m| m.archived_at.is_none())
                .max_by_key(|m| m.id)
                .map(|m| self.make_annotation_info(m)),
        };

        CaptureInfo {
            id: capture_model.id,
            user_id: capture_model.user_id,
            created_at: capture_model.created_at,
            created_at_human: common::humanize_datetime(capture_model.created_at),
            medias,
            illuminations,
            annotation,
        }
    }

    pub fn make_annotation_info(
        &self,
        annotation_model: model::annotation::ModelEx,
    ) -> AnnotationInfo {
        AnnotationInfo {
            id: annotation_model.id,
            capture_id: annotation_model.capture_id,
            content: annotation_model.content,
            created_at: annotation_model.created_at,
            updated_at: annotation_model.updated_at,
            archived_at: annotation_model.archived_at,
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
            HasMany::Loaded(models) => models.into_iter().map(KNodeInfo::from).collect(),
        };

        let social_medias = match illumination_model.social_medias {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models
                .into_iter()
                .map(SocialMediaInfo::from)
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

    pub fn make_capture_preview_info(
        &self,
        capture_model: &model::capture::ModelEx,
    ) -> Option<CapturePreviewInfo> {
        let media = match &capture_model.medias {
            HasMany::Unloaded => return None,
            HasMany::Loaded(models) => models.first()?,
        };

        let summary = match &capture_model.illuminations {
            HasMany::Unloaded => "No capture summary available.".to_string(),
            HasMany::Loaded(models) => models
                .first()
                .map(|illum| illum.summary.clone())
                .unwrap_or_else(|| "No capture summary available.".to_string()),
        };

        let handle = storage::StorageHandle::from(media);

        Some(CapturePreviewInfo {
            id: capture_model.id,
            url: self.url_maker.make_url(&handle),
            summary,
        })
    }

    pub fn make_spark_info(
        &self,
        spark_model: model::spark::ModelEx,
        capture_preview_map: &HashMap<i32, CapturePreviewInfo>,
    ) -> SparkInfo {
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
                .map(|m| self.make_spark_cluster_info(m, capture_preview_map))
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
            created_at: spark_model.created_at,
            created_at_human: common::humanize_datetime(spark_model.created_at),
            input_capture_ids,
            meta,
            spark_clusters,
        }
    }

    pub fn make_spark_cluster_info(
        &self,
        spark_cluster_model: model::spark_cluster::ModelEx,
        capture_preview_map: &HashMap<i32, CapturePreviewInfo>,
    ) -> SparkClusterInfo {
        let spark_links = match spark_cluster_model.spark_links {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(SparkLinkInfo::from).collect(),
        };

        let referenced_capture_ids = match spark_cluster_model.spark_output_refs {
            HasMany::Unloaded => vec![],
            HasMany::Loaded(models) => models.into_iter().map(|m| m.capture_id).collect(),
        };

        let capture_previews = referenced_capture_ids
            .iter()
            .filter_map(|capture_id| capture_preview_map.get(capture_id).cloned())
            .collect();

        SparkClusterInfo {
            id: spark_cluster_model.id,
            title: spark_cluster_model.title,
            summary: spark_cluster_model.summary,
            referenced_capture_ids,
            capture_previews,
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
