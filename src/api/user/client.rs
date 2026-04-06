use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::anyhow;

use crate::{api::*, auth, database, storage, task};

#[derive(Clone)]
pub struct UserApiClient {
    // TODO hack db is currently public for auth verification in rest/r_token.rs
    pub db: database::DbHandle,
    storage: Box<dyn storage::StorageProvider>,
    info_maker: InfoMaker,
    beacon: task::Beacon,
    capture_searcher: Option<super::CaptureSearcher>,
}

impl UserApiClient {
    pub fn new(
        db: database::DbHandle,
        storage: Box<dyn storage::StorageProvider>,
        url_maker: storage::UrlMaker,
        beacon: task::Beacon,
        capture_searcher: Option<super::CaptureSearcher>,
    ) -> Self {
        Self {
            db,
            storage,
            info_maker: schema::InfoMaker::new(url_maker),
            beacon,
            capture_searcher,
        }
    }

    #[tracing::instrument(skip(self, context, ids))]
    pub async fn get_captures(
        &self,
        context: &auth::Context,
        ids: Vec<i32>,
    ) -> Result<Vec<CaptureInfo>, ApiError> {
        let captures = super::get_captures(&self.db, context, ids).await;

        // TODO probably more efficient way here?
        Ok(captures?
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_illuminations(
        &self,
        context: &auth::Context,
        illumination_ids: Vec<i32>,
    ) -> Result<Vec<schema::IlluminationInfo>, ApiError> {
        let illuminations = super::get_illuminations(&self.db, context, illumination_ids).await?;

        Ok(illuminations
            .into_iter()
            .map(|m| self.info_maker.make_illumination_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_sparks(
        &self,
        context: &auth::Context,
        spark_ids: Option<Vec<i32>>,
    ) -> Result<Vec<schema::SparkInfo>, ApiError> {
        let sparks = super::get_sparks(&self.db, context, spark_ids).await?;

        self.make_spark_infos_from_models(context, sparks).await
    }

    async fn make_spark_infos_from_models(
        &self,
        context: &auth::Context,
        sparks: Vec<crate::model::spark::ModelEx>,
    ) -> Result<Vec<schema::SparkInfo>, ApiError> {
        let referenced_capture_ids: Vec<i32> = sparks
            .iter()
            .flat_map(|spark| match &spark.spark_clusters {
                sea_orm::prelude::HasMany::Unloaded => Vec::new(),
                sea_orm::prelude::HasMany::Loaded(clusters) => clusters
                    .iter()
                    .flat_map(|cluster| match &cluster.spark_output_refs {
                        sea_orm::prelude::HasMany::Unloaded => Vec::new(),
                        sea_orm::prelude::HasMany::Loaded(output_refs) => {
                            output_refs.iter().map(|r| r.capture_id).collect()
                        }
                    })
                    .collect(),
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let capture_preview_map: HashMap<i32, schema::CapturePreviewInfo> =
            if referenced_capture_ids.is_empty() {
                HashMap::new()
            } else {
                let captures =
                    super::get_captures(&self.db, context, referenced_capture_ids).await?;
                captures
                    .into_iter()
                    .filter_map(|capture| {
                        self.info_maker
                            .make_capture_preview_info(&capture)
                            .map(|preview| (capture.id, preview))
                    })
                    .collect()
            };

        Ok(sparks
            .into_iter()
            .map(|m| self.info_maker.make_spark_info(m, &capture_preview_map))
            .collect())
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_timeline_sparks(
        &self,
        context: &auth::Context,
        limit: u64,
    ) -> Result<Vec<schema::SparkInfo>, ApiError> {
        let sparks = super::get_timeline_sparks(&self.db, context, limit).await?;

        self.make_spark_infos_from_models(context, sparks).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_illumination_ids_need_search(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<i32>, ApiError> {
        super::get_illumination_ids_need_search(&self.db, context).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_knode(
        &self,
        context: &auth::Context,
        knode_id: i32,
    ) -> Result<schema::EntityInfo, ApiError> {
        let (knode, capture) = super::get_knode(&self.db, context, knode_id).await?;

        Ok(self.info_maker.make_knode_entity_info(knode, capture))
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_social_media(
        &self,
        context: &auth::Context,
        social_media_id: i32,
    ) -> Result<schema::EntityInfo, ApiError> {
        let (sm, capture) = super::get_social_media(&self.db, context, social_media_id).await?;

        Ok(self.info_maker.make_social_media_entity_info(sm, capture))
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_timeline_captures(
        &self,
        context: &auth::Context,
        limit: u64,
    ) -> Result<Vec<schema::CaptureInfo>, ApiError> {
        let captures = super::get_timeline_captures(&self.db, context, limit).await;

        Ok(captures?
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context, capture_ids))]
    pub async fn enqueue_spark(
        &self,
        context: &auth::Context,
        capture_ids: Vec<i32>,
    ) -> Result<(), ApiError> {
        if capture_ids.is_empty() {
            return Err(ApiError::bad_request(anyhow!(
                "capture_ids must contain at least one capture ID"
            )));
        }

        let requested_ids: HashSet<i32> = capture_ids.iter().copied().collect();
        let found = super::get_captures(&self.db, context, capture_ids.clone()).await?;
        let found_ids: HashSet<i32> = found.into_iter().map(|c| c.id).collect();

        if requested_ids != found_ids {
            let missing_ids: Vec<i32> = requested_ids.difference(&found_ids).copied().collect();

            tracing::warn!(
                requested_ids = ?requested_ids,
                found_ids = ?found_ids,
                missing_ids = ?missing_ids,
                "Some capture IDs not found or not accessible to user"
            );
        }

        self.beacon
            .signal_new_spark(capture_ids)
            .await
            .map_err(ApiError::internal)
    }

    /// import_capture has two novel behaviors compared to insert_capture:
    /// 1. It fails if the media hash already exists (to prevent duplicates)
    /// 2. It allows specifying the creation timestamp of the capture
    #[tracing::instrument(skip(self, user_context, media_bytes, created_at))]
    pub async fn import_capture(
        &self,
        user_context: &auth::Context,
        media_bytes: bytes::Bytes,
        created_at: DateTime<Utc>,
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model = super::insert_capture(
            &self.db,
            &self.storage,
            user_context,
            media_bytes,
            true, // fail on media dupes
            Some(created_at),
        )
        .await?;

        // TODO Should this live inside the inner insert_capture function instead?
        if let Err(e) = self.beacon.signal_new_capture(capture_model.id).await {
            tracing::warn!(
                capture_id = capture_model.id,
                error = ?e,
                "Ignoring error signaling new capture to beacon",
            );
        }

        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, user_context, media_bytes))]

    pub async fn insert_capture(
        &self,
        user_context: &auth::Context,
        media_bytes: bytes::Bytes,
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model = super::insert_capture(
            &self.db,
            &self.storage,
            user_context,
            media_bytes,
            false, // allow media dupes
            None,
        )
        .await?;

        // TODO Should this live inside the inner insert_capture function instead?
        if let Err(e) = self.beacon.signal_new_capture(capture_model.id).await {
            tracing::warn!(
                capture_id = capture_model.id,
                error = ?e,
                "Ignoring error signaling new capture to beacon",
            );
        }

        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn delete_capture(
        &self,
        context: &auth::Context,
        capture_id: i32,
    ) -> Result<(), ApiError> {
        super::delete_capture(&self.db, context, capture_id).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn archive_capture(
        &self,
        context: &auth::Context,
        capture_id: i32,
    ) -> Result<(), ApiError> {
        super::archive_capture(&self.db, context, capture_id).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn unarchive_capture(
        &self,
        context: &auth::Context,
        capture_id: i32,
    ) -> Result<(), ApiError> {
        super::unarchive_capture(&self.db, context, capture_id).await
    }

    #[tracing::instrument(skip(self, context, current_password, new_password))]
    pub async fn change_password(
        &self,
        context: &auth::Context,
        current_password: String,
        new_password: String,
    ) -> Result<(), ApiError> {
        super::change_password(&self.db, context, current_password, new_password).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn search(
        &self,
        context: &auth::Context,
        query: &str,
        limit: Option<u64>,
    ) -> Result<Vec<schema::CaptureInfo>, ApiError> {
        let capture_searcher = self.capture_searcher.as_ref().ok_or_else(|| {
            ApiError::internal(anyhow!(
                "Search backend unavailable: Gemini/Vertex not initialized from config"
            ))
        })?;

        let capture_ids = capture_searcher.search(context, query, limit).await?;
        if capture_ids.is_empty() {
            return Ok(vec![]);
        }

        let capture_models = super::get_captures(&self.db, context, capture_ids).await?;

        Ok(capture_models
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context), fields(capture_id = capture_id))]
    pub async fn search_similar(
        &self,
        context: &auth::Context,
        capture_id: i32,
        limit: Option<u64>,
    ) -> Result<Vec<schema::CaptureInfo>, ApiError> {
        let capture_searcher = self.capture_searcher.as_ref().ok_or_else(|| {
            ApiError::internal(anyhow!(
                "Search backend unavailable: Gemini/Vertex not initialized from config"
            ))
        })?;

        let mut query_capture_models = super::get_captures(&self.db, context, vec![capture_id]).await?;
        let Some(query_capture_model) = query_capture_models.pop() else {
            return Ok(vec![]);
        };
        let query_capture = self.info_maker.make_capture_info(query_capture_model);

        let capture_ids = capture_searcher
            .search_similar(context, &query_capture, limit)
            .await?;

        if capture_ids.is_empty() {
            return Ok(vec![]);
        }

        let capture_models = super::get_captures(&self.db, context, capture_ids).await?;

        Ok(capture_models
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }
}
