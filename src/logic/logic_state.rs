use crate::{api, database, ignition, llms, search, storage};

/// Dependencies used by background task logic.
pub struct LogicState {
    pub service_api: api::ServiceApiClient,
    /// The knowledge graph is written in-process, not through `service_api`: linking is a
    /// transaction over observations, entities and evidence, and there is no REST surface for it.
    pub db: database::DbHandle,
    pub storage: Box<dyn storage::StorageProvider>,
    pub inference_client: Box<dyn llms::InferenceClient>,
    pub firestarter: Box<dyn ignition::Firestarter>,
    pub embedder:
        Box<dyn search::Embedder<serde_json::Value, search::Embedding<f32, search::Unit>>>,
    pub vector_store: Box<dyn search::VectorStore<search::Embedding<f32, search::Unit>>>,
}
