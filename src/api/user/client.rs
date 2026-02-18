use std::sync::Arc;

use crate::{api::*, auth, database, storage, task};

#[derive(Clone)]
pub struct UserApiClient {
    // TODO hack db is currently public for auth verification in rest/r_token.rs
    pub db: database::DbHandle,
    storage: Box<dyn storage::StorageProvider>,
    info_maker: InfoMaker,
    task_publisher: Arc<dyn task::task_publisher::IlluminationTaskPublisher>,
}

impl UserApiClient {
    pub fn new(
        db: database::DbHandle,
        storage: Box<dyn storage::StorageProvider>,
        url_maker: storage::UrlMaker,
        task_publisher: Arc<dyn task::task_publisher::IlluminationTaskPublisher>,
    ) -> Self {
        Self {
            db,
            storage,
            info_maker: schema::InfoMaker::new(url_maker),
            task_publisher,
        }
    }

    #[tracing::instrument(skip(self, context, ids))]
    pub async fn get_captures(
        &self,
        context: &auth::Context,
        ids: Option<Vec<i32>>,
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
    pub async fn get_timeline(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<schema::CaptureInfo>, ApiError> {
        let captures = super::get_timeline(&self.db, context).await;

        Ok(captures?
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context, media_bytes))]
    pub async fn insert_capture(
        &self,
        context: &auth::Context,
        media_bytes: &[u8],
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model =
            super::insert_capture(&self.db, &self.storage, context, media_bytes).await?;

        if let Err(err) = self
            .task_publisher
            .publish_capture_id(capture_model.id)
            .await
        {
            tracing::error!(
                capture_id = capture_model.id,
                error = ?err,
                "Failed to publish illumination task; capture remains saved"
            );
        }

        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn search(
        &self,
        context: &auth::Context,
        query: &str,
    ) -> Result<Vec<schema::CaptureInfo>, ApiError> {
        let capture_models = super::search_by_illuminations(&self.db, context, query).await?;

        Ok(capture_models
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }
}
