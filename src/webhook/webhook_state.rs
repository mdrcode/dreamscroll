use crate::{api, ignition, illumination, search, storage};

pub struct WebhookState {
    pub service_api: api::ServiceApiClient,
    pub stg: Box<dyn storage::StorageProvider>,
    pub illuminator: Box<dyn illumination::Illuminator>,
    pub firestarter: Box<dyn ignition::Firestarter>,
    pub embedder: search::gcloud::GeminiEmbedder,
    pub vector_store: search::gcloud::VertexVectorStore,
}
