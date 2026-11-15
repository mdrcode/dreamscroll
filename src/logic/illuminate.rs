use serde::{Deserialize, Serialize};

use crate::{api, illumination, llms, task};

/// The concrete task for illuminating a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
    pub entity_owner_id: i32,
    /// Gemini model selected for this illumination run.
    pub model_id: String,
    /// Result schema and persistence path selected for this task.
    pub prompt_version: illumination::IlluminationVersion,
    /// Optional reasoning effort override. `None` delegates to Gemini's default, which is
    /// model-specific (`medium` for `gemini-3.8-flash`, `high` for `gemini-3.1-pro-preview`).
    /// See <https://ai.google.dev/gemini-api/docs/thinking#thinking-levels>.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<llms::gemini::ThinkingLevel>,
    /// Persist the relational projection and enqueue indexing; raw output is always retained.
    /// `new` starts at `true`; `false` is for testing or eval.
    pub persist_projection: bool,
    /// Enable the provider's web-search tool (Gemini `google_search` grounding).
    /// Billed per grounded request and roughly 30% slower. `new` starts at `false`.
    pub web_search: bool,
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
            thinking_level: None,
            persist_projection: true,
            web_search: false,
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

/// Executes the requested version. V1 is raw-only; v2 optionally writes the
/// relational KnowledgeEntity projection.
/// Returns the illumination it persisted a projection for, which is the work the knowledge graph
/// can link. `None` means nothing was projected (raw-only, or projection disabled).
pub async fn exec(
    state: &super::LogicState,
    task_run: &task::TaskRun<IlluminationTask>,
    attempt: i32,
) -> Result<Option<i32>, api::ApiError> {
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

    // Grounding is the only capability requested today, and the task stores it as a flag. Keep the
    // translation here so both prompt versions take the provider-facing slice unchanged.
    let capabilities: &[llms::InferenceCapability] = if task.web_search {
        &[llms::InferenceCapability::WebSearch]
    } else {
        &[]
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
                task.thinking_level,
                capabilities,
            )
            .await
            .map_err(api::ApiError::internal)?;

            let provider_status = illumination.http_status_code;
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
            let illumination = illumination.output.map_err(|error| {
                api::ApiError::internal(error)
                    .with_inner_status_code(provider_status.map(i32::from))
            })?;
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
                task.thinking_level,
                capabilities,
            )
            .await
            .map_err(api::ApiError::internal)?;

            let provider_status = illumination.http_status_code;
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
            let illumination = illumination.output.map_err(|error| {
                api::ApiError::internal(error)
                    .with_inner_status_code(provider_status.map(i32::from))
            })?;
            state
                .service_api
                .insert_illumination_raw(
                    &capture,
                    task_run_id,
                    inference_meta_id,
                    illumination.raw_json(),
                )
                .await?;
            if task.persist_projection {
                let illumination_id = state
                    .service_api
                    .insert_illumination_v2(&capture, task_run_id, inference_meta_id, illumination)
                    .await?;
                return Ok(Some(illumination_id));
            }
        }
    }

    tracing::info!(capture_id, task_run_id, prompt_version = ?task.prompt_version, persist_projection = task.persist_projection, "Illumination completed and persisted");

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tasks_are_projected_and_ungrounded() {
        let task = IlluminationTask::new(7, 1, "m", illumination::IlluminationVersion::V2);
        assert!(task.persist_projection);
        assert!(!task.web_search);

        // Both flags are always written, so a queued payload round-trips with no defaults.
        let queued: IlluminationTask =
            serde_json::from_value(serde_json::to_value(&task).unwrap()).unwrap();
        assert!(queued.persist_projection);
        assert!(!queued.web_search);
        assert_eq!(queued.capture_id, 7);
    }
}
