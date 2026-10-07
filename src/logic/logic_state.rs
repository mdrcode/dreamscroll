use crate::{api, ignition, llms, search, storage};

/// Dependencies used by background task logic.
pub struct LogicState {
    pub service_api: api::ServiceApiClient,
    pub storage: Box<dyn storage::StorageProvider>,
    pub gemini_client: llms::gemini::GeminiInferenceClient,
    pub firestarter: Box<dyn ignition::Firestarter>,
    pub embedder: search::gcloud::GeminiEmbedder,
    pub vector_store: search::gcloud::VertexVectorStore,
}
