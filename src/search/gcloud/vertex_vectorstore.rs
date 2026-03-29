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
    n_dims: usize,
}

impl VertexAiVectorStore {
    pub fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let index_id = config
            .search_embed_index_id
            .as_ref()
            .context("SEARCH_EMBED_INDEX_ID required for search indexing")?
            .to_string();
        let output_dims = config
            .search_embed_output_dims
            .context("SEARCH_EMBED_OUTPUT_DIMS required for search indexing")?
            as usize;

        Ok(Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            index_id,
            output_dims,
        ))
    }

    pub fn new(project_id: String, region: String, index_id: String, output_dims: usize) -> Self {
        let index_full_name = format!(
            "projects/{}/locations/{}/indexes/{}",
            project_id, region, index_id
        );

        Self {
            index_full_name,
            n_dims: output_dims,
        }
    }
}

#[async_trait::async_trait]
impl search::VectorStore for VertexAiVectorStore {
    #[tracing::instrument(skip(self, embed), fields(capture_id = %embed.capture_id, illumination_id = %embed.illumination_id))]
    async fn upsert_capture_embedding(
        &self,
        embed: &search::CaptureEmbedding,
    ) -> anyhow::Result<search::VectorUpsertResult> {
        if embed.embedding.len() != self.n_dims {
            anyhow::bail!(
                "Embedding dimension mismatch: expected {}, got {} (capture_id={}, illumination_id={})",
                self.n_dims,
                embed.embedding.len(),
                embed.capture_id,
                embed.illumination_id
            );
        }

        let index_client = IndexService::builder()
            .build()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to create IndexService client: {}", err))?;

        let restricts = vec![
            model::index_datapoint::Restriction::new()
                .set_namespace("user_id")
                .set_allow_list([embed.user_id.to_string()]),
            model::index_datapoint::Restriction::new()
                .set_namespace("capture_id")
                .set_allow_list([embed.capture_id.to_string()]),
        ];

        let datapoint_id = make_datapoint_id(embed);

        let datapoint = IndexDatapoint::new()
            .set_datapoint_id(datapoint_id.clone())
            .set_feature_vector(embed.embedding.clone())
            .set_restricts(restricts);

        let _response = index_client
            .upsert_datapoints()
            .set_index(self.index_full_name.clone())
            .set_datapoints([datapoint])
            .send()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to upsert vector datapoint: {}", err))?;

        tracing::info!(
            user_id = embed.user_id,
            capture_id = embed.capture_id,
            illumination_id = embed.illumination_id,
            datapoint_id,
            dimensions = embed.embedding.len(),
            "Vector datapoint upserted"
        );

        Ok(search::VectorUpsertResult {
            datapoint_id,
            embedding_dimensions: embed.embedding.len(),
        })
    }
}

fn make_datapoint_id(embed: &search::CaptureEmbedding) -> String {
    format!(
        "user_id:{}:capture_id:{}:illumination_id:{}",
        embed.user_id, embed.capture_id, embed.illumination_id
    )
}
