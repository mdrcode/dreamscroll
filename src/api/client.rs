use chrono::Utc;

use crate::{auth, database, illumination, storage};

use super::*;

#[derive(Clone)]
pub struct ApiClient {
    // TODO hack this is currently public for webui auth backend, fix later
    pub db: database::DbHandle,

    // TODO hack this is public for r_upload.rs, fix later
    pub storage: Box<dyn storage::StorageProvider>,
    info_maker: InfoMaker,
}

impl ApiClient {
    pub fn new(
        db: database::DbHandle,
        storage: Box<dyn storage::StorageProvider>,
        url_maker: storage::UrlMaker,
    ) -> Self {
        Self {
            db,
            storage,
            info_maker: InfoMaker::new(url_maker),
        }
    }

    #[tracing::instrument(skip(self, context, ids))]
    pub async fn get_captures(
        &self,
        context: &auth::Context,
        ids: Option<Vec<i32>>,
    ) -> Result<Vec<CaptureInfo>, ApiError> {
        let captures = get_captures(&self.db, context, ids).await;

        // TODO probably more efficient way here?
        Ok(captures?
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_captures_need_illum(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<i32>, ApiError> {
        get_captures_need_illum(&self.db, context).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_captures_need_search_idx(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<i32>, ApiError> {
        get_captures_need_search_idx(&self.db, context).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_timeline(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<CaptureInfo>, ApiError> {
        let captures = get_timeline(&self.db, context).await;

        Ok(captures?
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    #[tracing::instrument(skip(self, context, media1))]
    pub async fn insert_capture(
        &self,
        context: &auth::Context,
        media1: storage::StorageIdentity,
    ) -> Result<CaptureInfo, ApiError> {
        let capture_model = insert_capture(&self.db, context, media1).await?;

        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, context, capture, illumination))]
    pub async fn insert_illumination(
        &self,
        context: &auth::Context,
        capture: &CaptureInfo, // TODO could this just take capture id?
        illumination: illumination::Illumination,
    ) -> Result<(), ApiError> {
        insert_illumination(&self.db, context, capture, illumination).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn search(
        &self,
        context: &auth::Context,
        query: &str,
    ) -> Result<Vec<CaptureInfo>, ApiError> {
        let capture_models = search_by_illuminations(&self.db, context, query).await?;

        Ok(capture_models
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    // TODO will move this to its own ImportClient facade later
    #[tracing::instrument(skip(self, context, media1, created_at))]
    pub async fn import_capture(
        &self,
        context: &auth::Context,
        media1: storage::StorageIdentity,
        created_at: chrono::DateTime<Utc>,
    ) -> Result<CaptureInfo, ApiError> {
        let capture_model = import::import_capture(&self.db, context, media1, created_at).await?;
        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, media))]
    pub async fn get_media_storage(&self, media: MediaInfo) -> Result<Vec<u8>, ApiError> {
        get_media_bytes(&self.storage, media).await
    }
}
