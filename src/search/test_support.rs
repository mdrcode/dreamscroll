use super::{DataObject, Embedder, Embedding, Unit, VectorStore, VectorUpsertResult};

pub(crate) struct UnusedEmbedder;

#[async_trait::async_trait]
impl Embedder<serde_json::Value, Embedding<f32, Unit>> for UnusedEmbedder {
    async fn embed_query(&self, _query: &str) -> anyhow::Result<Embedding<f32, Unit>> {
        anyhow::bail!("search is not used by the v2 worker test")
    }

    async fn embed_object(
        &self,
        _object: serde_json::Value,
    ) -> anyhow::Result<Embedding<f32, Unit>> {
        anyhow::bail!("search is not used by the v2 worker test")
    }
}

pub(crate) struct UnusedVectorStore;

#[async_trait::async_trait]
impl VectorStore<Embedding<f32, Unit>> for UnusedVectorStore {
    async fn upsert_object_embedding(
        &self,
        _object: &dyn DataObject,
        _embedding: &Embedding<f32, Unit>,
    ) -> anyhow::Result<VectorUpsertResult> {
        anyhow::bail!("search is not used by the v2 worker test")
    }

    async fn fetch_object_embedding(
        &self,
        _object_id: &str,
    ) -> anyhow::Result<Option<Embedding<f32, Unit>>> {
        anyhow::bail!("search is not used by the v2 worker test")
    }
}
