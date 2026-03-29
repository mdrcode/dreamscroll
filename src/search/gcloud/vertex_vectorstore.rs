use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectService,
    model::{DataObject, DenseVector, Vector},
};
use serde_json::json;

use crate::{facility, search};

const VECTOR_FIELD_NAME: &str = "dense_hybrid_v1";

enum UpdateOutcome {
    Updated,
    NeedsInsert,
    Error(google_cloud_vectorsearch_v1::Error),
}

/// Upserts dense vectors into Vertex Vector Search 2.0 Collections.
#[derive(Clone)]
pub struct VertexAiVectorStore {
    collection_full_path: String,
    n_dims: usize,
}

impl VertexAiVectorStore {
    pub fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let collection_id = config
            .search_embed_collection_id
            .as_ref()
            .context("SEARCH_EMBED_COLLECTION_ID required for search indexing")?
            .to_string();
        let output_dims = config
            .search_embed_output_dims
            .context("SEARCH_EMBED_OUTPUT_DIMS required for search indexing")?
            as usize;

        Ok(Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            collection_id,
            output_dims,
        ))
    }

    pub fn new(
        project_id: String,
        region: String,
        collection_id: String,
        output_dims: usize,
    ) -> Self {
        let collection_full_name = format!(
            "projects/{}/locations/{}/collections/{}",
            project_id, region, collection_id
        );

        tracing::info!(
            collection_full_name,
            output_dims,
            "VertexAiVectorStore initialized"
        );

        Self {
            collection_full_path: collection_full_name,
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
                "Dimension mismatch: VectorStore dims: {}, embedding: {:?}",
                self.n_dims,
                embed
            );
        }

        let data_object_client = DataObjectService::builder()
            .build()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to create DataObjectService client: {}", err))?;

        let object_id = make_data_object_id(embed);
        let object_full_path = format!("{}/dataObjects/{}", self.collection_full_path, object_id);

        let data_object = DataObject::new()
            .set_name(object_full_path.clone())
            .set_data(
                json!({
                    "user_id": embed.user_id,
                    "capture_id": embed.capture_id,
                    "illumination_id": embed.illumination_id,
                })
                .as_object()
                .cloned()
                .expect("json object"),
            )
            .set_vectors(make_vectors(embed));

        let update_outcome = match data_object_client
            .update_data_object()
            .set_data_object(data_object.clone())
            .send()
            .await
        {
            Ok(_) => UpdateOutcome::Updated,
            Err(update_err)
                if update_err
                    .status()
                    .is_some_and(|status| status.code as i32 == 5)
                    || update_err.http_status_code() == Some(404) =>
            {
                UpdateOutcome::NeedsInsert
            }
            Err(update_err) => UpdateOutcome::Error(update_err),
        };

        let operation = match update_outcome {
            UpdateOutcome::Updated => "updated",
            UpdateOutcome::NeedsInsert => {
                data_object_client
                    .create_data_object()
                    .set_parent(self.collection_full_path.clone())
                    .set_data_object_id(object_id.clone())
                    .set_data_object(data_object)
                    .send()
                    .await
                    .map_err(|create_err| {
                        anyhow::anyhow!(
                            "Failed to create vector data object after update miss: {}",
                            create_err
                        )
                    })?;

                "created"
            }
            UpdateOutcome::Error(update_err) => {
                anyhow::bail!(
                    "Failed to upsert vector data object via update: {}",
                    update_err
                );
            }
        };

        tracing::info!(
            user_id = embed.user_id,
            capture_id = embed.capture_id,
            illumination_id = embed.illumination_id,
            collection = self.collection_full_path,
            object_id,
            vector_field = VECTOR_FIELD_NAME,
            operation,
            dimensions = embed.embedding.len(),
            "Vector data object upserted"
        );

        Ok(search::VectorUpsertResult {
            id: object_id,
            fq_id: Some(object_full_path),
            dims: embed.embedding.len(),
        })
    }
}

fn make_data_object_id(embed: &search::CaptureEmbedding) -> String {
    format!(
        "u{}-c{}-i{}",
        embed.user_id, embed.capture_id, embed.illumination_id
    )
}

fn make_vectors(embed: &search::CaptureEmbedding) -> Vec<(String, Vector)> {
    vec![(
        VECTOR_FIELD_NAME.to_string(),
        Vector::new().set_dense(DenseVector::new().set_values(embed.embedding.clone())),
    )]
}
