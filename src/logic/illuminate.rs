use serde::{Deserialize, Serialize};

use crate::{api, illumination, llms, task};

/// The concrete task for illuminating a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
    pub entity_owner_id: i32,
    /// Gemini model selected for this illumination run.
    pub model_id: String,
    /// Provider-specific request controls: thinking level, storage, summaries, grounding.
    pub gemini: llms::gemini::GeminiInferenceOptions,
    /// Persist the relational projection and enqueue indexing; raw output is always retained.
    /// `new` starts at `true`; `false` is for testing or eval.
    pub persist_projection: bool,
}

impl IlluminationTask {
    pub fn new(capture_id: i32, entity_owner_id: i32, model_id: impl Into<String>) -> Self {
        Self {
            capture_id,
            entity_owner_id,
            model_id: model_id.into(),
            gemini: llms::gemini::GeminiInferenceOptions::default(),
            persist_projection: true,
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

/// Runs v2 illumination, optionally writing the relational KnowledgeEntity projection.
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

    let illumination = illumination::v2::illuminate(
        state.inference_client.as_ref(),
        state.storage.as_ref(),
        &capture,
        &task.model_id,
        &task.gemini,
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
        api::ApiError::internal(error).with_inner_status_code(provider_status.map(i32::from))
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

    tracing::info!(
        capture_id,
        task_run_id,
        persist_projection = task.persist_projection,
        "Illumination completed and persisted"
    );

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tasks_are_projected_and_ungrounded() {
        let task = IlluminationTask::new(7, 1, "m");
        assert!(task.persist_projection);
        assert!(!task.gemini.web_search);
        assert!(!task.gemini.store);
        assert!(!task.gemini.thinking_summaries);

        // The whole task, including options, round-trips as queued work.
        let queued: IlluminationTask =
            serde_json::from_value(serde_json::to_value(&task).unwrap()).unwrap();
        assert!(queued.persist_projection);
        assert!(!queued.gemini.web_search);
        assert!(!queued.gemini.store);
        assert!(!queued.gemini.thinking_summaries);
        assert_eq!(queued.capture_id, 7);
    }
}
