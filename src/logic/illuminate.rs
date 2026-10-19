use serde::{Deserialize, Serialize};

use crate::{api, illumination, task};

/// The concrete task for illuminating a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
    pub entity_owner_id: i32,
    /// Gemini model selected for this illumination run.
    pub model_id: String,
    /// Result schema and persistence path selected for this task.
    pub prompt_version: illumination::IlluminationVersion,
}

impl IlluminationTask {
    pub fn new(
        capture_id: i32,
        entity_owner_id: i32,
        model_id: impl Into<String>,
        prompt_version: illumination::IlluminationVersion,
    ) -> Self {
        Self {
            capture_id,
            entity_owner_id,
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
    fn entity_owner_id(&self) -> i32 {
        self.entity_owner_id
    }

    fn task_type() -> &'static str {
        "illuminate"
    }
}

/// Executes the requested version. V1 is raw-only; v2 also writes the
/// relational KnowledgeEntity projection.
pub async fn exec(
    state: &super::LogicState,
    task_run: &task::TaskRun<IlluminationTask>,
    attempt: i32,
) -> Result<(), api::ApiError> {
    let task = &task_run.task;
    let task_run_id = task_run.run_id.as_str();
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

    match task.prompt_version {
        illumination::IlluminationVersion::V1 => {
            tracing::warn!(
                capture_id,
                "V1 illumination is raw-only; no relational projection or search index will be written"
            );
            let illumination = illumination::v1::illuminate(
                state.inference_client.as_ref(),
                state.storage.as_ref(),
                &capture,
                &task.model_id,
            )
            .await
            .map_err(api::ApiError::internal)?;

            let inference_meta_id = state
                .service_api
                .record_inference(
                    capture.user_id,
                    task_run_id,
                    attempt,
                    <IlluminationTask as task::Task>::task_type(),
                    illumination.metadata,
                )
                .await;
            let illumination = illumination.output.map_err(api::ApiError::internal)?;
            state
                .service_api
                .insert_illumination_raw(
                    &capture,
                    task_run_id,
                    inference_meta_id,
                    illumination.raw_json(),
                )
                .await?;
        }
        illumination::IlluminationVersion::V2 => {
            let illumination = illumination::v2::illuminate(
                state.inference_client.as_ref(),
                state.storage.as_ref(),
                &capture,
                &task.model_id,
            )
            .await
            .map_err(api::ApiError::internal)?;

            let inference_meta_id = state
                .service_api
                .record_inference(
                    capture.user_id,
                    task_run_id,
                    attempt,
                    <IlluminationTask as task::Task>::task_type(),
                    illumination.metadata,
                )
                .await;
            let illumination = illumination.output.map_err(api::ApiError::internal)?;
            state
                .service_api
                .insert_illumination_raw(
                    &capture,
                    task_run_id,
                    inference_meta_id,
                    illumination.raw_json(),
                )
                .await?;
            state
                .service_api
                .insert_illumination_v2(&capture, task_run_id, inference_meta_id, illumination)
                .await?;
        }
    }

    tracing::info!(capture_id, task_run_id, prompt_version = ?task.prompt_version, "Illumination completed and persisted");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_payload_serializes_prompt_version_and_model() {
        let task = IlluminationTask::new(123, 1, "model-a", illumination::IlluminationVersion::V1);
        let payload = serde_json::to_value(task).expect("task should serialize");

        assert_eq!(payload["capture_id"], 123);
        assert_eq!(payload["entity_owner_id"], 1);
        assert_eq!(payload["model_id"], "model-a");
        assert_eq!(payload["prompt_version"], "v1");
    }
}
