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
        url_maker: storage::StorageUrlMaker,
    ) -> Self {
        Self {
            db,
            storage,
            info_maker: InfoMaker::new(url_maker),
        }
    }

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

    pub async fn get_captures_need_illum(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<i32>, ApiError> {
        get_captures_need_illum(&self.db, context).await
    }

    pub async fn get_captures_need_search_idx(
        &self,
        user: &auth::Context,
    ) -> Result<Vec<i32>, ApiError> {
        get_captures_need_search_idx(&self.db, user).await
    }

    pub async fn get_timeline(
        &self,
        user_context: &auth::Context,
    ) -> Result<Vec<CaptureInfo>, ApiError> {
        let captures = get_timeline(&self.db, user_context).await;

        Ok(captures?
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    pub async fn insert_capture(
        &self,
        user_context: &auth::Context,
        media1: storage::StorageIdentity,
    ) -> Result<CaptureInfo, ApiError> {
        let capture_model = insert_capture(&self.db, user_context, media1).await?;

        Ok(self.info_maker.make_capture_info(capture_model))
    }

    pub async fn insert_illumination(
        &self,
        context: &auth::Context,
        capture: &CaptureInfo,
        illumination: illumination::Illumination,
    ) -> Result<(), ApiError> {
        insert_illumination(&self.db, context, capture, illumination).await
    }

    pub async fn search(
        &self,
        user_context: &auth::Context,
        query: &str,
    ) -> Result<Vec<CaptureInfo>, ApiError> {
        let capture_models = search_by_illuminations(&self.db, user_context, query).await?;

        Ok(capture_models
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    // TODO will move this to its own ImportClient facade later
    pub async fn import_capture(
        &self,
        user_context: &auth::Context,
        media1: storage::StorageIdentity,
        created_at: chrono::DateTime<Utc>,
    ) -> Result<CaptureInfo, ApiError> {
        let capture_model =
            import::import_capture(&self.db, user_context, media1, created_at).await?;
        Ok(self.info_maker.make_capture_info(capture_model))
    }

    pub async fn get_media_storage(&self, media: MediaInfo) -> Result<Vec<u8>, ApiError> {
        get_media_storage(&self.storage, media).await
    }
}
