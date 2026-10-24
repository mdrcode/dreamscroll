use std::sync::Arc;

use crate::{api, ignition, llms, search, storage};

/// Dependencies used by background task logic.
pub struct LogicState {
    pub service_api: api::ServiceApiClient,
    pub storage: Box<dyn storage::StorageProvider>,
    pub inference_client: Box<dyn llms::InferenceClient>,
    pub firestarter: Box<dyn ignition::Firestarter>,
    /// Shared so background indexing and search use the same instance for the Local backend.
    pub search_service: Arc<search::SearchService>,
}
