use chrono::Utc;

use crate::{auth, database, illumination, storage};

use super::{ApiError, import, schema};

#[derive(Clone)]
pub struct ApiClient {
    // TODO hack this is currently public for auth verification in rest/r_token.rs
    pub db: database::DbHandle,

    // TODO hack this is public for r_upload.rs, fix later
    pub storage: Box<dyn storage::StorageProvider>,
    info_maker: schema::InfoMaker,
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
            info_maker: schema::InfoMaker::new(url_maker),
        }
    }

    #[tracing::instrument(skip(self, context, ids))]
    pub async fn get_captures(
        &self,
        context: &auth::Context,
        ids: Option<Vec<i32>>,
    ) -> Result<Vec<schema::CaptureInfo>, ApiError> {
        let captures = super::get_captures(&self.db, context, ids).await;

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
        super::get_captures_need_illum(&self.db, context).await
    }

    #[tracing::instrument(skip(self, context))]
    pub async fn get_captures_need_search_idx(
        &self,
        context: &auth::Context,
    ) -> Result<Vec<i32>, ApiError> {
        super::get_captures_need_search_idx(&self.db, context).await
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

    #[tracing::instrument(skip(self, context, media1))]
    pub async fn insert_capture(
        &self,
        context: &auth::Context,
        media1: storage::StorageIdentity,
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model = super::insert_capture(&self.db, context, media1).await?;

        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, context, capture, illumination))]
    pub async fn insert_illumination(
        &self,
        context: &auth::Context,
        capture: &schema::CaptureInfo, // TODO could this just take capture id?
        illumination: illumination::Illumination,
    ) -> Result<(), ApiError> {
        super::insert_illumination(&self.db, context, capture, illumination).await
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

    // TODO will move this to its own ImportClient facade later
    #[tracing::instrument(skip(self, context, media1, created_at))]
    pub async fn import_capture(
        &self,
        context: &auth::Context,
        media1: storage::StorageIdentity,
        created_at: chrono::DateTime<Utc>,
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model = import::import_capture(&self.db, context, media1, created_at).await?;
        Ok(self.info_maker.make_capture_info(capture_model))
    }

    #[tracing::instrument(skip(self, media))]
    pub async fn get_media_storage(&self, media: schema::MediaInfo) -> Result<Vec<u8>, ApiError> {
        super::get_media_bytes(&self.storage, media).await
    }
}
