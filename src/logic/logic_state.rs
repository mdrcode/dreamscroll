use crate::{api, fetch, ignition, llms, search, storage};

/// Dependencies used by background task logic.
pub struct LogicState {
    pub storage: Box<dyn storage::StorageProvider>,
    pub service_api: api::ServiceApiClient,
    pub cache: fetch::FetchCache,

    pub inference_client: Box<dyn llms::InferenceClient>,
    pub firestarter: Box<dyn ignition::Firestarter>,

    pub knowledge_linker: crate::knowledge::LinkHelper,
    pub knowledge_identifier: crate::knowledge::IdentifyHelper,
    pub wikidata_allowance: fetch::AllowanceConfig,

    pub embedder:
        Box<dyn search::Embedder<serde_json::Value, search::Embedding<f32, search::Unit>>>,
    pub vector_store: Box<dyn search::VectorStore<search::Embedding<f32, search::Unit>>>,
}
