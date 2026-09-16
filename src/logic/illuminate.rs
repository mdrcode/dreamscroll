use serde::{Deserialize, Serialize};

use crate::{api, illumination, search, storage, task};

/// The concrete task for illuminating a single capture.
///
/// This is the app's core unit of work: illumination is only useful if the
/// result is also searchable, so `exec` runs **both** the illumination and the
/// search-indexing steps. See `exec` for details.
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

/// Illuminate a capture, then index it for search.
///
/// Illumination has no real purpose without search indexing, so the two are a
/// single unit of work: every illumination is followed by an index update.
/// Both steps are individually idempotent, so a retry (or a redelivery) is
/// safe:
///
/// 1. `illuminate_capture` — skips if the capture already has an illumination.
/// 2. `search_index::exec` — skips if the embedding already exists.
///
/// Note that step 2 runs **even when step 1 was skipped**. That matters: a
/// capture whose illumination succeeded but whose indexing failed on a previous
/// attempt must still get indexed on retry.
pub async fn exec(
    service_api: &api::ServiceApiClient,
    illuminator: &dyn illumination::Illuminator,
    stg: &dyn storage::StorageProvider,
    embedder: &search::gcloud::GeminiEmbedder,
    vector_store: &search::gcloud::VertexVectorStore,
    task: IlluminationTask,
) -> Result<(), api::ApiError> {
    illuminate_capture(service_api, illuminator, task.capture_id).await?;

    super::search_index::exec(
        service_api,
        stg,
        embedder,
        vector_store,
        super::search_index::SearchIndexTask {
            capture_id: task.capture_id,
        },
    )
    .await?;

    tracing::info!(
        capture_id = task.capture_id,
        "Illumination + search indexing completed"
    );

    Ok(())
}

/// The illumination half of `exec`. Idempotent: returns `Ok(())` without doing
/// anything if the capture already has an illumination.
async fn illuminate_capture(
    service_api: &api::ServiceApiClient,
    illuminator: &dyn illumination::Illuminator,
    capture_id: i32,
) -> Result<(), api::ApiError> {
    tracing::Span::current().record("capture_id", capture_id);

    let fetch = service_api.get_captures(Some(vec![capture_id])).await?;

    let Some(capture) = fetch.into_iter().next() else {
        tracing::warn!(capture_id, "Capture not found during illumination");
        return Ok(());
    };

    if !capture.illuminations.is_empty() {
        tracing::info!(
            capture_id,
            illumination_count = capture.illuminations.len(),
            "Idempotency guard: illumination already exists for capture; skipping"
        );
        return Ok(());
    }

    let illumination = illuminator.illuminate(&capture).await?;

    service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(capture_id, "Illumination completed and inserted");

    Ok(())
}
