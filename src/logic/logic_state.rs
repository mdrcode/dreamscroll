use crate::{api, fetch, ignition, llms, search, storage};

/// Dependencies used by background task logic.
pub struct LogicState {
    pub storage: Box<dyn storage::StorageProvider>,
    pub service_api: api::ServiceApiClient,
    pub cache: fetch::FetchCache,

    pub knowledge: crate::knowledge2::LinkHelper,
    pub identity_engine: crate::knowledge2::IdentifyHelper,
    /// What one attempt may spend at Wikidata: transport calls and the pace between them.\
    pub wikidata_allowance: fetch::AllowanceConfig,

    pub inference_client: Box<dyn llms::InferenceClient>,
    pub firestarter: Box<dyn ignition::Firestarter>,

    pub embedder:
        Box<dyn search::Embedder<serde_json::Value, search::Embedding<f32, search::Unit>>>,
    pub vector_store: Box<dyn search::VectorStore<search::Embedding<f32, search::Unit>>>,
}
