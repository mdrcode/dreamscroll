use crate::{
    api,
    search::{self, prelude::*},
    storage, webhook,
};

pub async fn exec(
    service_api: &api::ServiceApiClient,
    stg: &dyn storage::StorageProvider,
    embedder: &search::gcloud::GeminiEmbedder,
    vector_store: &search::gcloud::VertexVectorStore,
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
    let already_indexed = vector_store
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

    let embed_input = search::make_capture_info_embed_input(stg, &capture).await?;

    let embedding = embedder
        .embed_object(embed_input)
        .await
        .map_err(api::ApiError::internal)?;

    let upsert_result = vector_store
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
