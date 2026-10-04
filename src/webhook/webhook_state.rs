use std::sync::Arc;

use crate::{api, ignition, illumination, logic, search, storage, task};

pub struct WebhookState {
    pub service_api: api::ServiceApiClient,
    pub stg: Box<dyn storage::StorageProvider>,
    pub illuminator: Box<dyn illumination::Illuminator>,
    pub beacon: logic::Beacon,
    pub firestarter: Box<dyn ignition::Firestarter>,
    pub embedder: search::gcloud::GeminiEmbedder,
    pub vector_store: search::gcloud::VertexVectorStore,
    pub task_master: Arc<task::TaskMaster>,
}
