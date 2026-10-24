use serde::{Deserialize, Serialize};

use crate::{api, search, task};

/// The concrete task for search-indexing a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexTask {
    pub capture_id: i32,
    pub entity_owner_id: i32,
}

impl task::Task for SearchIndexTask {
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
        "search_index"
    }
}

pub async fn exec(
    state: &super::LogicState,
    task_run: &task::TaskRun<SearchIndexTask>,
    attempt: i32,
) -> Result<(), api::ApiError> {
    let task = &task_run.task;
    let task_run_id = task_run.run_id.as_str();
    tracing::Span::current().record("capture_id", task.capture_id);

    let fetch = state
        .service_api
        .get_captures(Some(vec![task.capture_id]))
        .await?;

    let Some(capture) = fetch.into_iter().next() else {
        tracing::warn!(
            capture_id = task.capture_id,
            "Capture not found during search indexing"
        );
        return Ok(());
    };

    if capture.illuminations.is_empty() {
        tracing::error!(
            capture_id = task.capture_id,
            "Capture has no illuminations yet; failing search indexing so task can retry"
        );
        return Err(api::ApiError::internal(anyhow::anyhow!(
            "capture_id={} has no illuminations",
            task.capture_id
        )));
    }

    let embed_input =
        search::make_capture_info_embed_input(state.storage.as_ref(), &capture).await?;

    let inference = state
        .search_service
        .embedder()
        .embed_object(embed_input)
        .await
        .map_err(api::ApiError::internal)?;
    state
        .service_api
        .record_inference(
            capture.user_id,
            task_run_id,
            attempt,
            <SearchIndexTask as task::Task>::task_type(),
            inference.metadata,
        )
        .await;
    let embedding = inference.output.map_err(api::ApiError::internal)?;
    let upsert_result = state
        .search_service
        .vector_store()
        .upsert_object_embedding(&capture, &embedding)
        .await
        .map_err(api::ApiError::internal)?;

    tracing::info!(
        capture_id = task.capture_id,
        vector_id = upsert_result.id,
        dims = upsert_result.dims,
        "Search indexing completed"
    );

    Ok(())
}
