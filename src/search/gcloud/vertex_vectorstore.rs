use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectService,
    model::{DataObject, DenseVector, Vector},
};

use crate::{facility, search};

/// Upserts dense vectors into Vertex Vector Search 2.0 Collections.
#[derive(Clone)]
pub struct VertexVectorStore {
    collection_full_path: String,
    dense_vector_name: String,
    dense_vector_dims: usize,
    data_object_client: DataObjectService,
}

impl VertexVectorStore {
    pub async fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let collection_id = config
            .search_embed_collection_id
            .as_ref()
            .context("SEARCH_EMBED_COLLECTION_ID required for search indexing")?
            .to_string();
        let dense_vector_name = config
            .search_embed_vector_field
            .as_ref()
            .context("SEARCH_EMBED_VECTOR_FIELD required for search indexing")?
            .to_string();
        let output_dims = config
            .search_embed_vector_dims
            .context("SEARCH_EMBED_VECTOR_DIMS required for search indexing")?
            as usize;

        Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            collection_id,
            dense_vector_name,
            output_dims,
        )
        .await
    }

    pub async fn new(
        project_id: String,
        region: String,
        collection_id: String,
        dense_vector_name: String,
        dense_vector_dims: usize,
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
            dense_vector_name,
            dense_vector_dims,
            "VertexVectorStore initialized"
        );

        Ok(Self {
            collection_full_path: collection_full_name,
            dense_vector_name,
            dense_vector_dims,
            data_object_client,
        })
    }
}

#[async_trait::async_trait]
impl search::VectorStore<search::Embedding<f32, search::Unit>> for VertexVectorStore {
    #[tracing::instrument(skip(self, data, embedding), fields(doc_id = data.data_object_id()))]
    async fn upsert_object_embedding(
        &self,
        data: &dyn search::DataObject,
        embedding: &search::Embedding<f32, search::Unit>,
    ) -> anyhow::Result<search::VectorUpsertResult> {
        if embedding.len() != self.dense_vector_dims {
            anyhow::bail!(
                "Dimension mismatch: VectorStore dims: {}, embedding: {:?}",
                self.dense_vector_dims,
                embedding
            );
        }

        let object_id = data.data_object_id();
        let object_full_path = format!("{}/dataObjects/{}", self.collection_full_path, object_id);
        let object_data = data.data_object_json()?;

        let data_object = DataObject::new()
            .set_name(object_full_path.clone())
            .set_data(object_data)
            .set_vectors(vec![(
                self.dense_vector_name.clone(),
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
            dims: self.dense_vector_dims,
        })
    }

    async fn fetch_object_embedding(
        &self,
        object_id: &str,
    ) -> anyhow::Result<Option<search::Embedding<f32, search::Unit>>> {
        let object_name = format!("{}/dataObjects/{}", self.collection_full_path, object_id);

        let data_object = match self
            .data_object_client
            .get_data_object()
            .set_name(object_name.clone())
            .send()
            .await
        {
            Ok(data_object) => data_object,
            Err(err) if not_found(&err) => return Ok(None),
            Err(err) => {
                anyhow::bail!(
                    "Failed to fetch vector data object {}: {}",
                    object_name,
                    err
                )
            }
        };

        let vector = data_object
            .vectors
            .get(&self.dense_vector_name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Data object {} missing vector field '{}'",
                    object_name,
                    self.dense_vector_name
                )
            })?;

        let dense = vector.dense().ok_or_else(|| {
            anyhow::anyhow!(
                "Data object {} vector field '{}' is not dense",
                object_name,
                self.dense_vector_name
            )
        })?;

        let embedding = search::Embedding::from_vec_normalizing(dense.values.clone())?;
        Ok(Some(embedding))
    }
}

// NOTE: We intentionally use numeric gRPC codes here (5/6) for now.
// `google_cloud_vectorsearch_v1` uses newer gax internals where typed rpc::Code
// enums exist, but our direct `google-cloud-gax` dependency version does not
// expose that API at this path yet. Revisit after aligning gax crate versions.
fn not_found(err: &google_cloud_vectorsearch_v1::Error) -> bool {
    err.status().is_some_and(|status| status.code as i32 == 5)
        || err.http_status_code() == Some(404)
}

fn already_exists(err: &google_cloud_vectorsearch_v1::Error) -> bool {
    err.status().is_some_and(|status| status.code as i32 == 6)
        || err.http_status_code() == Some(409)
}
