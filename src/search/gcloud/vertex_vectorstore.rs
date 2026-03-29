use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectService,
    model::{DataObject, DenseVector, Vector},
};
use serde_json::json;

use crate::{facility, search};

const VECTOR_FIELD_NAME: &str = "dense_hybrid_v1";

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

        // this is full-clobber overwrite; can consider update-make in the future
        let operation = match self
            .data_object_client
            .update_data_object()
            .set_data_object(data_object.clone())
            .send()
            .await
        {
            Ok(_) => "updated",
            Err(update_err) => {
                let not_found = update_err
                    .status()
                    .is_some_and(|status| status.code as i32 == 5)
                    || update_err.http_status_code() == Some(404);

                if !not_found {
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
                    Err(create_err)
                        if create_err
                            .status()
                            .is_some_and(|status| status.code as i32 == 6)
                            || create_err.http_status_code() == Some(409) =>
                    {
                        tracing::warn!(
                            object_id,
                            "Create after update miss returned AlreadyExists; treating as concurrent upsert success"
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
