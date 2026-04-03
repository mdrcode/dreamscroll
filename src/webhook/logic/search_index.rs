use crate::{
    api,
    search::{self, DataObject, Embedder, VectorStore},
    storage, webhook,
};

#[derive(Clone)]
pub struct SearchIndexer {
    embedder: search::gcloud::GeminiEmbedder<api::CaptureInfo, api::CaptureInfoEmbedPartsMaker>,
    vector_store: search::gcloud::VertexVectorStore,
}

impl SearchIndexer {
    pub async fn from_config(
        config: &crate::facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> Option<Self> {
        let parts_maker = api::CaptureInfoEmbedPartsMaker::new(storage);
        let embedder = match search::gcloud::GeminiEmbedder::from_config(config, parts_maker) {
            Ok(embedder) => embedder,
            Err(err) => {
                tracing::warn!(error = %err, "GeminiEmbedder init failed for SearchIndexer");
                return None;
            }
        };

        let vector_store = match search::gcloud::VertexVectorStore::from_config(config).await {
            Ok(vector_store) => vector_store,
            Err(err) => {
                tracing::warn!(error = %err, "VertexVectorStore init failed for SearchIndexer");
                return None;
            }
        };

        Some(Self {
            embedder,
            vector_store,
        })
    }
}

pub async fn exec(
    service_api: &api::ServiceApiClient,
    search_indexer: &SearchIndexer,
    task: webhook::schema::SearchIndexTask,
) -> Result<(), api::ApiError> {
    tracing::Span::current().record("capture_id", task.capture_id);

    let fetch = service_api
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

    let object_id = capture.data_object_id();
    let already_indexed = search_indexer
        .vector_store
        .fetch_object_embedding(&object_id)
        .await
        .map_err(api::ApiError::internal)?
        .is_some();
    if already_indexed {
        tracing::info!(
            capture_id = task.capture_id,
            object_id,
            "Idempotency guard: embedding already exists; skipping search indexing"
        );
        return Ok(());
    }

    let embedding = search_indexer
        .embedder
        .embed_object(&capture)
        .await
        .map_err(api::ApiError::internal)?;

    let upsert_result = search_indexer
        .vector_store
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
