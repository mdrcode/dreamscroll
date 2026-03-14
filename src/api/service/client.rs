use crate::{api::*, database, ignition, illumination, storage};

#[derive(Clone)]
pub struct ServiceApiClient {
    db: database::DbHandle,
    info_maker: InfoMaker,
}

impl ServiceApiClient {
    pub fn new(db: database::DbHandle, info_maker: storage::UrlMaker) -> Self {
        Self {
            db,
            info_maker: InfoMaker::new(info_maker),
        }
    }

    pub async fn get_captures(&self, ids: Option<Vec<i32>>) -> Result<Vec<CaptureInfo>, ApiError> {
        let capture_models = super::get_captures(&self.db, ids).await?;

        Ok(capture_models
            .into_iter()
            .map(|m| self.info_maker.make_capture_info(m))
            .collect())
    }

    pub async fn get_captures_need_illum(&self) -> Result<Vec<i32>, ApiError> {
        super::get_captures_need_illum(&self.db).await
    }

    pub async fn insert_illumination(
        &self,
        capture_info: &schema::CaptureInfo, // TODO could this just take capture id?
        illumination: illumination::Illumination,
    ) -> Result<(), ApiError> {
        super::insert_illumination(&self.db, capture_info, illumination).await
    }

    pub async fn insert_spark(
        &self,
        user_id: i32,
        input_capture_ids: Vec<i32>,
        spark: ignition::SparkResponse,
        meta: ignition::SparkMeta,
    ) -> Result<(), ApiError> {
        super::insert_spark(&self.db, user_id, input_capture_ids, spark, meta).await
    }
}
