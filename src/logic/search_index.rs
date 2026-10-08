use serde::{Deserialize, Serialize};

use crate::{api, search, task};

/// The concrete task for search-indexing a single capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexTask {
    pub capture_id: i32,
}

impl task::Task for SearchIndexTask {
    fn entity_type() -> &'static str {
        "capture"
    }

    fn entity_id(&self) -> i32 {
        self.capture_id
    }

    fn task_type() -> &'static str {
        "search_index"
    }
}

pub async fn exec(state: &super::LogicState, task: SearchIndexTask) -> Result<(), api::ApiError> {
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

    let upsert_result = embed_and_upsert(
        &capture,
        embed_input,
        state.embedder.as_ref(),
        state.vector_store.as_ref(),
    )
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

async fn embed_and_upsert(
    capture: &api::CaptureInfo,
    embed_input: serde_json::Value,
    embedder: &dyn search::Embedder<serde_json::Value, search::Embedding<f32, search::Unit>>,
    vector_store: &dyn search::VectorStore<search::Embedding<f32, search::Unit>>,
) -> anyhow::Result<search::VectorUpsertResult> {
    let embedding = embedder.embed_object(embed_input).await?;
    vector_store
        .upsert_object_embedding(capture, &embedding)
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct TestEmbedder {
        seen_input: Mutex<Option<serde_json::Value>>,
    }

    #[async_trait::async_trait]
    impl search::Embedder<serde_json::Value, search::Embedding<f32, search::Unit>> for TestEmbedder {
        async fn embed_query(
            &self,
            _query: &str,
        ) -> anyhow::Result<search::Embedding<f32, search::Unit>> {
            anyhow::bail!("query embedding is not used by search indexing")
        }

        async fn embed_object(
            &self,
            object: serde_json::Value,
        ) -> anyhow::Result<search::Embedding<f32, search::Unit>> {
            *self.seen_input.lock().unwrap() = Some(object);
            search::Embedding::from_vec_normalizing(vec![3.0, 4.0])
        }
    }

    #[derive(Default)]
    struct TestVectorStore {
        seen_object_id: Mutex<Option<String>>,
        seen_embedding: Mutex<Option<Vec<f32>>>,
    }

    #[async_trait::async_trait]
    impl search::VectorStore<search::Embedding<f32, search::Unit>> for TestVectorStore {
        async fn upsert_object_embedding(
            &self,
            object: &dyn search::DataObject,
            embedding: &search::Embedding<f32, search::Unit>,
        ) -> anyhow::Result<search::VectorUpsertResult> {
            let id = object.data_object_id();
            *self.seen_object_id.lock().unwrap() = Some(id.clone());
            *self.seen_embedding.lock().unwrap() = Some(embedding.as_slice().to_vec());
            Ok(search::VectorUpsertResult {
                id,
                fq_id: None,
                dims: embedding.len(),
            })
        }

        async fn fetch_object_embedding(
            &self,
            _object_id: &str,
        ) -> anyhow::Result<Option<search::Embedding<f32, search::Unit>>> {
            Ok(None)
        }
    }

    fn capture_info() -> api::CaptureInfo {
        api::CaptureInfo {
            id: 42,
            user_id: 7,
            created_at: chrono::Utc::now(),
            created_at_human: String::new(),
            medias: Vec::new(),
            illuminations: Vec::new(),
            annotation: None,
        }
    }

    #[tokio::test]
    async fn embed_and_upsert_passes_capture_and_embedding_to_vector_store() {
        let capture = capture_info();
        let embedder = TestEmbedder::default();
        let vector_store = TestVectorStore::default();
        let input = serde_json::json!({"text": "illumination text"});

        let result = embed_and_upsert(&capture, input.clone(), &embedder, &vector_store)
            .await
            .unwrap();

        assert_eq!(*embedder.seen_input.lock().unwrap(), Some(input));
        assert_eq!(
            *vector_store.seen_object_id.lock().unwrap(),
            Some("u7-c42".to_string())
        );
        assert_eq!(
            *vector_store.seen_embedding.lock().unwrap(),
            Some(vec![0.6, 0.8])
        );
        assert_eq!(result.id, "u7-c42");
        assert_eq!(result.dims, 2);
    }
}
