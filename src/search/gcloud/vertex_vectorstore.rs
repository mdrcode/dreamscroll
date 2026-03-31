use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectService,
    model::{DataObject, DenseVector, Vector},
};
use serde_json::json;

use crate::{facility, search};

use super::*;

/// Upserts dense vectors into Vertex Vector Search 2.0 Collections.
#[derive(Clone)]
pub struct VertexAiVectorStore {
    collection_full_path: String,
    n_dims: usize,
    data_object_client: DataObjectService,
}

impl VertexAiVectorStore {
    pub async fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let collection_id = config
            .search_embed_collection_id
            .as_ref()
            .context("SEARCH_EMBED_COLLECTION_ID required for search indexing")?
            .to_string();
        let output_dims = config
            .search_embed_output_dims
            .context("SEARCH_EMBED_OUTPUT_DIMS required for search indexing")?
            as usize;

        Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            collection_id,
            output_dims,
        )
        .await
    }

    pub async fn new(
        project_id: String,
        region: String,
        collection_id: String,
        output_dims: usize,
    ) -> anyhow::Result<Self> {
        let collection_full_name = format!(
            "projects/{}/locations/{}/collections/{}",
            project_id, region, collection_id
        );

        let data_object_client = DataObjectService::builder()
            .build()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to create DataObjectService client: {}", err))?;

        tracing::info!(
            collection_full_name,
            output_dims,
            "VertexAiVectorStore initialized"
        );

        Ok(Self {
            collection_full_path: collection_full_name,
            n_dims: output_dims,
            data_object_client,
        })
    }
}

#[async_trait::async_trait]
impl search::VectorStore<search::Embedding<f32, search::Unit>> for VertexAiVectorStore {
    #[tracing::instrument(skip(self, embed), fields(capture_id = %embed.capture_id, illumination_id = %embed.illumination_id))]
    async fn upsert_capture_embedding(
        &self,
        embed: &search::CaptureEmbedding<search::Embedding<f32, search::Unit>>,
    ) -> anyhow::Result<search::VectorUpsertResult> {
        let embedding = &embed.embedding;

        if embedding.len() != self.n_dims {
            anyhow::bail!(
                "Dimension mismatch: VectorStore dims: {}, embedding: {:?}",
                self.n_dims,
                embed
            );
        }

        let object_id = data_object_id::make(embed);
        let object_full_path = format!("{}/dataObjects/{}", self.collection_full_path, object_id);

        let data_object = DataObject::new()
            .set_name(object_full_path.clone())
            .set_data(
                // note that ID fields are strings (matching schema_vertex_data.json)
                json!({
                    "user_id": embed.user_id.to_string(),
                    "capture_id": embed.capture_id.to_string(),
                    "illumination_id": embed.illumination_id.to_string(),
                    "illumination_text": embed.illumination_text,
                })
                .as_object()
                .cloned()
                .expect("data_object json"),
            )
            .set_vectors(vec![(
                constants::CAPTURE_DENSE_VECTOR.to_string(),
                Vector::new()
                    .set_dense(DenseVector::new().set_values(embedding.as_slice().to_vec())),
            )]);

        // Try update first, then create.
        // Note: full-clobber overwrite; TODO consider partial field-wise update
        let update_result = self
            .data_object_client
            .update_data_object()
            .set_data_object(data_object.clone())
            .send()
            .await;

        let operation = match update_result {
            Ok(_) => "updated",
            Err(update_err) if not_found(&update_err) => {
                let create_result = self
                    .data_object_client
                    .create_data_object()
                    .set_parent(self.collection_full_path.clone())
                    .set_data_object_id(object_id.clone())
                    .set_data_object(data_object)
                    .send()
                    .await;

                match create_result {
                    Ok(_) => "created",
                    Err(create_err) if already_exists(&create_err) => {
                        tracing::warn!(
                            object_id,
                            "Create after update miss returned AlreadyExists"
                        );
                        "already_exists"
                    }
                    Err(create_err) => {
                        tracing::error!(
                            error = %create_err,
                            object_id,
                            "Create failed for vector data object after update miss"
                        );
                        anyhow::bail!(
                            "Failed to create vector data object after update miss: {}",
                            create_err
                        );
                    }
                }
            }
            Err(update_err) => {
                tracing::error!(
                    error = %update_err,
                    object_id,
                    "Update failed for vector data object"
                );
                anyhow::bail!(
                    "Failed to upsert vector data object via update: {}",
                    update_err
                );
            }
        };

        tracing::info!(
            collection = self.collection_full_path,
            object_id,
            operation,
            "Vector data object upserted"
        );

        Ok(search::VectorUpsertResult {
            id: object_id,
            fq_id: Some(object_full_path),
            dims: self.n_dims,
        })
    }
}

fn not_found(err: &google_cloud_vectorsearch_v1::Error) -> bool {
    err.status().is_some_and(|status| status.code as i32 == 5)
        || err.http_status_code() == Some(404)
}

fn already_exists(err: &google_cloud_vectorsearch_v1::Error) -> bool {
    err.status().is_some_and(|status| status.code as i32 == 6)
        || err.http_status_code() == Some(409)
}
