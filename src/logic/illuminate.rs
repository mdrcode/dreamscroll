use serde::{Deserialize, Serialize};

use crate::{api, illumination, task};

/// The concrete task for illuminating a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
}

impl task::Task for IlluminationTask {
    fn entity_type() -> &'static str {
        "c"
    }

    fn entity_id(&self) -> i32 {
        self.capture_id
    }

    fn task_type() -> &'static str {
        "illuminate"
    }
}

pub async fn exec(
    service_api: &api::ServiceApiClient,
    illuminator: &dyn illumination::Illuminator,
    task: IlluminationTask,
) -> Result<(), api::ApiError> {
    tracing::Span::current().record("capture_id", task.capture_id);

    let fetch = service_api
        .get_captures(Some(vec![task.capture_id]))
        .await?;

    let Some(capture) = fetch.into_iter().next() else {
        tracing::warn!(
            capture_id = task.capture_id,
            "Capture not found during illumination"
        );
        return Ok(());
    };

    if !capture.illuminations.is_empty() {
        tracing::info!(
            capture_id = task.capture_id,
            illumination_count = capture.illuminations.len(),
            "Idempotency guard: illumination already exists for capture; skipping"
        );
        return Ok(());
    }

    let illumination = illuminator.illuminate(&capture).await?;

    service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(
        capture_id = task.capture_id,
        "Illumination completed and inserted"
    );

    Ok(())
}
