use serde::{Deserialize, Serialize};

use crate::{api, illumination, task};

/// The concrete task for illuminating a single capture.
///
/// Performs illumination and, on success, signals Beacon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
}

impl task::Task for IlluminationTask {
    fn entity_type() -> &'static str {
        "capture"
    }

    fn entity_id(&self) -> i32 {
        self.capture_id
    }

    fn task_type() -> &'static str {
        "illuminate"
    }
}

/// Illuminate a capture and signal Beacon after successful persistence.
///
/// Deliberately **not** idempotent. A retry re-illuminates; we
/// tolerate the duplicate API calls rather than carry guard logic that would
/// also have to be made rerun-aware (a "skip if already illuminated" check
/// silently no-ops every rerun). Reruns append an illumination per run, and
/// `InfoMaker` collapses them to the most recent for display.
pub async fn exec(
    service_api: &api::ServiceApiClient,
    illuminator: &dyn illumination::Illuminator,
    beacon: &super::Beacon,
    task: IlluminationTask,
) -> Result<(), api::ApiError> {
    illuminate_capture(service_api, illuminator, beacon, task.capture_id).await
}

async fn illuminate_capture(
    service_api: &api::ServiceApiClient,
    illuminator: &dyn illumination::Illuminator,
    beacon: &super::Beacon,
    capture_id: i32,
) -> Result<(), api::ApiError> {
    tracing::Span::current().record("capture_id", capture_id);

    let fetch = service_api.get_captures(Some(vec![capture_id])).await?;

    let Some(capture) = fetch.into_iter().next() else {
        tracing::warn!(capture_id, "Capture not found during illumination");
        return Ok(());
    };

    let illumination = illuminator.illuminate(&capture).await?;

    service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(capture_id, "Illumination completed and inserted");

    beacon
        .new_illumination(capture.user_id, capture_id)
        .await
        .map_err(api::ApiError::internal)?;

    Ok(())
}
