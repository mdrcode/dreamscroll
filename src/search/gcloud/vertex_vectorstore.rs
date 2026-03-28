use anyhow::Context;
use google_cloud_aiplatform_v1::{
    client::IndexService,
    model::{self, IndexDatapoint},
};

use crate::{facility, search};

/// Upserts dense vectors into Vertex AI Vector Search.
#[derive(Clone)]
pub struct VertexAiVectorStore {
    project_id: String,
    region: String,
    vector_index_id: String,
}

impl VertexAiVectorStore {
    pub fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let vector_index_id = config
            .search_vector_index_id
            .as_ref()
            .context("SEARCH_VECTOR_INDEX_ID required for search indexing")?
            .to_string();

        Ok(Self {
            project_id: config.gcloud_project_id.clone(),
            region: config.gcloud_project_region.clone(),
            vector_index_id,
        })
    }

    pub fn new(project_id: String, region: String, vector_index_id: String) -> Self {
        Self {
            project_id,
            region,
            vector_index_id,
        }
    }

    async fn upsert_embedding_impl(
        &self,
        embedded: &search::CaptureEmbedding,
    ) -> anyhow::Result<()> {
        let index_full_name = format!(
            "projects/{}/locations/{}/indexes/{}",
            self.project_id, self.region, self.vector_index_id
        );

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

        let datapoint = IndexDatapoint::new()
            .set_datapoint_id(embedded.datapoint_id.clone())
            .set_feature_vector(embedded.embedding.clone())
            .set_restricts(restricts);

        index_client
            .upsert_datapoints()
            .set_index(index_full_name)
            .set_datapoints([datapoint])
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to upsert vector datapoint: {}", e))?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl search::VectorStore for VertexAiVectorStore {
    async fn upsert_capture_embedding(
        &self,
        embedded: &search::CaptureEmbedding,
    ) -> anyhow::Result<search::VectorUpsertResult> {
        self.upsert_embedding_impl(embedded).await?;

        Ok(search::VectorUpsertResult {
            datapoint_id: embedded.datapoint_id.clone(),
            embedding_dimensions: embedded.embedding.len(),
        })
    }
}
