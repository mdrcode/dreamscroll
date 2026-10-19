use crate::{api::*, database, ignition, illumination, llms, storage};

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

    pub async fn record_inference(
        &self,
        user_id: i32,
        task_run_id: &str,
        attempt: i32,
        task_type: &str,
        metadata: llms::InferenceMetadata,
    ) -> Option<i32> {
        super::record_inference(&self.db, user_id, task_run_id, attempt, task_type, metadata).await
    }

    pub async fn insert_illumination_v2(
        &self,
        capture_info: &schema::CaptureInfo,
        task_run_id: &str,
        inference_meta_id: Option<i32>,
        illumination: illumination::v2::Illumination,
    ) -> Result<(), ApiError> {
        super::insert_illumination_v2(
            &self.db,
            capture_info,
            task_run_id,
            inference_meta_id,
            illumination,
        )
        .await
    }

    pub async fn insert_illumination_raw(
        &self,
        capture_info: &schema::CaptureInfo,
        task_run_id: &str,
        inference_meta_id: Option<i32>,
        raw: &serde_json::Value,
    ) -> Result<(), ApiError> {
        super::insert_illumination_raw(&self.db, capture_info, task_run_id, inference_meta_id, raw)
            .await
    }

    pub async fn insert_spark(
        &self,
        user_id: i32,
        input_capture_ids: Vec<i32>,
        spark: ignition::SparkResponse,
        inference_meta_id: Option<i32>,
    ) -> Result<(), ApiError> {
        super::insert_spark(
            &self.db,
            user_id,
            input_capture_ids,
            spark,
            inference_meta_id,
        )
        .await
    }
}
