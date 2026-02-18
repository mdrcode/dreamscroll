use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::{api::*, auth, database, storage};

pub struct ImportApiClient {
    db: database::DbHandle,
    storage: Box<dyn storage::StorageProvider>,
    info_maker: InfoMaker,
}

impl ImportApiClient {
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

    #[tracing::instrument(skip(self, user_context, created_at, path))]
    pub async fn import_capture(
        &self,
        user_context: &auth::Context,
        created_at: DateTime<Utc>,
        path: &PathBuf,
    ) -> Result<schema::CaptureInfo, ApiError> {
        let capture_model =
            super::import_capture(&self.db, &self.storage, &user_context, created_at, path).await?;
        Ok(self.info_maker.make_capture_info(capture_model))
    }
}
