use crate::api;

use super::Illuminator;

#[derive(Clone)]
pub struct CaptureIlluminationProcessor {
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn Illuminator>,
}

impl CaptureIlluminationProcessor {
    pub fn new(service_api: api::ServiceApiClient, illuminator: Box<dyn Illuminator>) -> Self {
        Self {
            service_api,
            illuminator,
        }
    }

    pub async fn process_capture_id(&self, capture_id: i32) -> anyhow::Result<(), api::ApiError> {
        let fetch = self.service_api.get_captures(Some(vec![capture_id])).await?;

        let Some(capture) = fetch.into_iter().next() else {
            tracing::warn!(capture_id, "Capture not found during illumination");
            return Ok(());
        };

        let illumination = match self.illuminator.illuminate(&capture).await {
            Ok(value) => value,
            Err(err) => {
                tracing::error!(
                    capture_id,
                    error = ?err,
                    "Illumination model call failed for capture"
                );
                return Err(api::ApiError::internal(err));
            }
        };

        self.service_api
            .insert_illumination(&capture, illumination)
            .await?;

        tracing::info!(capture_id, "Illumination completed and inserted");
        Ok(())
    }

    pub async fn get_captures_need_illum(&self) -> anyhow::Result<Vec<i32>, api::ApiError> {
        self.service_api.get_captures_need_illum().await
    }
}
