use chrono::{DateTime, Utc};

use crate::{api::*, database, storage};

pub struct ImportApiClient {
    db: database::DbHandle,
    info_maker: InfoMaker,
}

impl ImportApiClient {
    pub fn new(db: database::DbHandle, url_maker: storage::UrlMaker) -> Self {
        Self {
            db,
            info_maker: InfoMaker::new(url_maker),
        }
    }

    #[tracing::instrument(skip(self, media1, created_at))]
    pub async fn import_capture(
        &self,
        user_id: i32,
        media1: storage::StorageHandle,
        created_at: DateTime<Utc>,
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model = super::import_capture(&self.db, user_id, media1, created_at).await?;
        Ok(self.info_maker.make_capture_info(capture_model))
    }
}
