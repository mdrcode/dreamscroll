use serde::{Deserialize, Serialize};

use crate::{api, illumination, search, storage, task};

/// The concrete task for ingesting a single capture: illuminate + search-index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPayload {
    pub capture_id: i32,
}

impl task::TaskPayload for IngestPayload {
    fn task_type() -> &'static str {
        "ingest"
    }
}

/// Run the full ingest pipeline for a capture: illumination, then search indexing.
pub async fn exec(
    service_api: &api::ServiceApiClient,
    illuminator: &dyn illumination::Illuminator,
    stg: &dyn storage::StorageProvider,
    embedder: &search::gcloud::GeminiEmbedder,
    vector_store: &search::gcloud::VertexVectorStore,
    task: IngestPayload,
) -> Result<(), api::ApiError> {
    // Illuminate
    super::illuminate::exec(
        service_api,
        illuminator,
        super::illuminate::IlluminationPayload {
            capture_id: task.capture_id,
        },
    )
    .await?;

    // Index for Search
    super::search_index::exec(
        service_api,
        stg,
        embedder,
        vector_store,
        super::search_index::SearchIndexPayload {
            capture_id: task.capture_id,
        },
    )
    .await?;

    tracing::info!(
        capture_id = task.capture_id,
        "Ingest completed: illumination + search indexing"
    );

    Ok(())
}
