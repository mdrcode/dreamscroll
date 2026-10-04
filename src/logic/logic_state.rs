use crate::{api, ignition, illumination, search, storage};

/// Dependencies used by background task logic.
pub struct LogicState {
    pub service_api: api::ServiceApiClient,
    pub storage: Box<dyn storage::StorageProvider>,
    pub illuminator: Box<dyn illumination::Illuminator>,
    pub firestarter: Box<dyn ignition::Firestarter>,
    pub embedder: search::gcloud::GeminiEmbedder,
    pub vector_store: search::gcloud::VertexVectorStore,
}
