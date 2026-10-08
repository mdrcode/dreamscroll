use serde::{Deserialize, Serialize};

use crate::{api, illumination, task};

/// The concrete task for illuminating a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
    /// Gemini model selected for this illumination run.
    pub model_id: String,
    /// Result schema and persistence path selected for this task.
    pub prompt_version: illumination::IlluminationVersion,
}

impl IlluminationTask {
    pub fn new(
        capture_id: i32,
        model_id: impl Into<String>,
        prompt_version: illumination::IlluminationVersion,
    ) -> Self {
        Self {
            capture_id,
            model_id: model_id.into(),
            prompt_version,
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

/// Executes the requested version. Both results are persisted raw; v1 also writes
/// the current relational projection, while v2 remains an evaluation-only path.
///
/// Each call creates a concrete inference ID and returns it with the capture owner.
/// Retries re-illuminate with new IDs; settled reruns append a projection.
pub async fn exec(
    state: &super::LogicState,
    task: &IlluminationTask,
) -> Result<(i32, task::TaskRunResultRef), api::ApiError> {
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

    let inference_id = uuid::Uuid::new_v4().to_string();
    match task.prompt_version {
        illumination::IlluminationVersion::V1 => {
            let illumination = illumination::v1::illuminate(
                &state.gemini_client,
                state.storage.as_ref(),
                &capture,
                &task.model_id,
                inference_id.clone(),
            )
            .await
            .map_err(api::ApiError::internal)?;

            state
                .service_api
                .insert_illumination_raw(&capture, &illumination)
                .await?;
            state
                .service_api
                .insert_illumination_v1(&capture, illumination)
                .await?;
        }
        illumination::IlluminationVersion::V2 => {
            let illumination = illumination::v2::illuminate(
                &state.gemini_client,
                state.storage.as_ref(),
                &capture,
                &task.model_id,
                inference_id.clone(),
            )
            .await
            .map_err(api::ApiError::internal)?;

            state
                .service_api
                .insert_illumination_raw(&capture, &illumination)
                .await?;
            tracing::warn!(
                capture_id,
                "Only persisting v2::Illumination result in illumination_raw at this time"
            );
        }
    }

    tracing::info!(capture_id, prompt_version = ?task.prompt_version, "Illumination completed and persisted");

    Ok((
        capture.user_id,
        task::TaskRunResultRef::new("inference", inference_id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_payload_serializes_prompt_version_and_model() {
        let task = IlluminationTask::new(123, "model-a", illumination::IlluminationVersion::V1);
        let payload = serde_json::to_value(task).expect("task should serialize");

        assert_eq!(payload["capture_id"], 123);
        assert_eq!(payload["model_id"], "model-a");
        assert_eq!(payload["prompt_version"], "v1");
    }
}
