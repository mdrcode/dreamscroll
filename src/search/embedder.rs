#[async_trait::async_trait]
pub trait Embedder<O, E>: Send + Sync {
    async fn embed_query(&self, query: &str) -> anyhow::Result<E>;
    async fn embed_object(&self, object: O) -> anyhow::Result<E>;
}
