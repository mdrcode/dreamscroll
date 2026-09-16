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
///
/// Deliberately **not** idempotent. A retry re-illuminates and re-indexes; we
/// tolerate the duplicate API calls rather than carry guard logic that would
/// also have to be made rerun-aware (a "skip if already illuminated" check
/// silently no-ops every rerun). Reruns append an illumination per run, and
/// `InfoMaker` collapses them to the most recent for display.
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

/// The illumination half of `exec`.
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

    let illumination = illuminator.illuminate(&capture).await?;

    service_api
        .insert_illumination(&capture, illumination)
        .await?;

    tracing::info!(capture_id, "Illumination completed and inserted");

    Ok(())
}
