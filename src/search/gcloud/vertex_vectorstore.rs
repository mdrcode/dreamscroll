use anyhow::Context;
use google_cloud_aiplatform_v1::{
    client::IndexService,
    model::{self, IndexDatapoint},
};

use crate::{facility, search};

/// Upserts dense vectors into Vertex AI Vector Search.
#[derive(Clone)]
pub struct VertexAiVectorStore {
    index_full_name: String,
}

impl VertexAiVectorStore {
    pub fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let index_id = config
            .search_vector_index_id
            .as_ref()
            .context("SEARCH_VECTOR_INDEX_ID required for search indexing")?
            .to_string();

        Ok(Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            index_id,
        ))
    }

    pub fn new(project_id: String, region: String, index_id: String) -> Self {
        let index_full_name = format!(
            "projects/{}/locations/{}/indexes/{}",
            project_id, region, index_id
        );

        Self { index_full_name }
    }
}

#[async_trait::async_trait]
impl search::VectorStore for VertexAiVectorStore {
    async fn upsert_capture_embedding(
        &self,
        embedded: &search::CaptureEmbedding,
    ) -> anyhow::Result<search::VectorUpsertResult> {
        let index_client = IndexService::builder()
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create IndexService client: {}", e))?;

        let restricts = vec![
            model::index_datapoint::Restriction::new()
                .set_namespace("user_id")
                .set_allow_list([embedded.user_id.to_string()]),
            model::index_datapoint::Restriction::new()
                .set_namespace("capture_id")
                .set_allow_list([embedded.capture_id.to_string()]),
        ];

        let datapoint_id = make_datapoint_id(embedded.capture_id, embedded.illumination_id);

        let datapoint = IndexDatapoint::new()
            .set_datapoint_id(datapoint_id.clone())
            .set_feature_vector(embedded.embedding.clone())
            .set_restricts(restricts);

        index_client
            .upsert_datapoints()
            .set_index(self.index_full_name.clone())
            .set_datapoints([datapoint])
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to upsert vector datapoint: {}", e))?;

        Ok(search::VectorUpsertResult {
            datapoint_id,
            embedding_dimensions: embedded.embedding.len(),
        })
    }
}

fn make_datapoint_id(capture_id: i32, illumination_id: i32) -> String {
    format!("capture:{}:illumination:{}", capture_id, illumination_id)
}
