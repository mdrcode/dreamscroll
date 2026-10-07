use serde::{Deserialize, Serialize};

use crate::{api, illumination, task};

/// The concrete task for illuminating a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
    /// Gemini model selected for this illumination run.
    pub model_id: String,
}

impl IlluminationTask {
    pub fn new(capture_id: i32, model_id: impl Into<String>) -> Self {
        Self {
            capture_id,
            model_id: model_id.into(),
        }
    }
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

/// Illuminate a capture and persist the result.
///
/// Deliberately **not** idempotent. A retry re-illuminates; we
/// tolerate the duplicate API calls rather than carry guard logic that would
/// also have to be made rerun-aware (a "skip if already illuminated" check
/// silently no-ops every rerun). Reruns append an illumination per run, and
/// `InfoMaker` collapses them to the most recent for display.
pub async fn exec(
    state: &super::LogicState,
    task: IlluminationTask,
    inference_run_id: String,
) -> Result<i32, api::ApiError> {
    let capture_id = task.capture_id;
    tracing::Span::current().record("capture_id", capture_id);

    let fetch = state
        .service_api
        .get_captures(Some(vec![capture_id]))
        .await?;

    let Some(capture) = fetch.into_iter().next() else {
        return Err(api::ApiError::not_found(anyhow::anyhow!(
            "Capture {capture_id} not found during illumination"
        )));
    };

    let illumination = illumination::v1::illuminate(
        &state.gemini_client,
        state.storage.as_ref(),
        &capture,
        &task.model_id,
        inference_run_id,
    )
    .await
    .map_err(api::ApiError::internal)?;

    state
        .service_api
        .insert_illumination_raw(&capture, &illumination)
        .await?;

    state
        .service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(capture_id, "Illumination completed and inserted");

    Ok(capture.user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_payload_serializes_selected_model() {
        let task = IlluminationTask::new(123, "model-a");
        let payload = serde_json::to_value(task).expect("task should serialize");

        assert_eq!(payload["capture_id"], 123);
        assert_eq!(payload["model_id"], "model-a");
    }
}
