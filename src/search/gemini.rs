use anyhow::Context;
use base64::Engine;
use google_cloud_aiplatform_v1::{
    client::IndexService,
    model::{self, IndexDatapoint},
};
use google_cloud_auth::credentials::Builder;
use reqwest::Client;
use serde_json::{Value, json};

use crate::{api, facility, storage};

use super::*;

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const DEFAULT_EMBEDDING_MODEL: &str = "gemini-embedding-2-preview";
const DEFAULT_VERTEX_LOCATION: &str = "us-central1";

/// Indexes a capture by building a single hybrid image+text embedding via
/// Gemini Embeddings v2 and upserting it into Vertex AI Vector Search.
#[derive(Clone)]
pub struct GeminiV2SearchIndexer {
    project_id: String,
    location: String,
    embedding_model: String,
    vector_index_id: String,
    output_dimensionality: Option<u32>,
    storage: Box<dyn storage::StorageProvider>,
    http: Client,
    adc_credentials: google_cloud_auth::credentials::AccessTokenCredentials,
}

impl GeminiV2SearchIndexer {
    pub fn from_config(
        config: &facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> anyhow::Result<Self> {
        let vector_index_id = config
            .search_vector_index_id
            .as_ref()
            .context("SEARCH_VECTOR_INDEX_ID is required for search indexing")?
            .to_string();

        let location = config.search_vertex_location.clone().unwrap_or_else(|| {
            if config.gcloud_project_region == "local" {
                DEFAULT_VERTEX_LOCATION.to_string()
            } else {
                config.gcloud_project_region.clone()
            }
        });

        let embedding_model = config
            .search_embedding_model
            .clone()
            .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());

        Self::new(
            &config.gcloud_project_id,
            &location,
            &embedding_model,
            &vector_index_id,
            config.search_embedding_output_dimensionality,
            storage,
        )
    }

    pub fn new(
        project_id: &str,
        location: &str,
        embedding_model: &str,
        vector_index_id: &str,
        output_dimensionality: Option<u32>,
        storage: Box<dyn storage::StorageProvider>,
    ) -> anyhow::Result<Self> {
        let adc_credentials = Builder::default()
            .with_scopes([CLOUD_PLATFORM_SCOPE])
            .build_access_token_credentials()?;

        Ok(Self {
            project_id: project_id.to_string(),
            location: location.to_string(),
            embedding_model: embedding_model.to_string(),
            vector_index_id: vector_index_id.to_string(),
            output_dimensionality,
            storage,
            http: reqwest::Client::new(),
            adc_credentials,
        })
    }

    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    pub async fn index_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<SearchIndexUpsertResult> {
        let media = capture
            .medias
            .first()
            .context("Capture has no media; cannot embed for search")?;

        let illumination = capture
            .illuminations
            .first()
            .context("Capture has no illumination; search embedding requires text context")?;

        let hybrid_doc = docmaker::make_hybrid_document(capture, media, illumination)?;
        let image_bytes = self
            .storage
            .retrieve_bytes(&storage::StorageHandle::from(media))
            .await?;
        let embedding = self
            .embed_hybrid_document(&hybrid_doc, &image_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!("Embedding request failed for capture {}: {}", capture.id, e)
            })?;

        let datapoint_id = self.make_datapoint_id(capture.id, illumination.id);
        self.upsert_embedding(capture, &datapoint_id, embedding.clone())
            .await?;

        tracing::info!(
            capture_id = capture.id,
            illumination_id = illumination.id,
            datapoint_id,
            dimensions = embedding.len(),
            "Search embedding indexed in Vertex Vector Search"
        );

        Ok(SearchIndexUpsertResult {
            datapoint_id,
            embedding_dimensions: embedding.len(),
        })
    }

    async fn embed_hybrid_document(
        &self,
        doc: &HybridSearchDocument,
        image_bytes: &[u8],
    ) -> anyhow::Result<Vec<f32>> {
        let access_token = self.adc_credentials.access_token().await?.token;
        let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:embedContent",
            self.location, self.project_id, self.location, self.embedding_model
        );

        let mut body = json!({
            "content": {
                "parts": [
                    {
                        "text": doc.text
                    },
                    {
                        "inline_data": {
                            "mime_type": doc.mime_type,
                            "data": image_b64
                        }
                    }
                ]
            }
        });

        if let Some(d) = self.output_dimensionality {
            body["config"] = json!({
                "output_dimensionality": d,
            });
        }

        let response = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Gemini embedContent failed with status {} and body: {}",
                status,
                body
            );
        }

        let value: serde_json::Value = response.json().await?;
        parse_embedding_values(&value)
    }

    async fn upsert_embedding(
        &self,
        capture: &api::CaptureInfo,
        datapoint_id: &str,
        feature_vector: Vec<f32>,
    ) -> anyhow::Result<()> {
        let index_full_name = format!(
            "projects/{}/locations/{}/indexes/{}",
            self.project_id, self.location, self.vector_index_id
        );

        let index_client = IndexService::builder()
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create IndexService client: {}", e))?;

        let restricts = vec![
            model::index_datapoint::Restriction::new()
                .set_namespace("user_id")
                .set_allow_list([capture.user_id.to_string()]),
            model::index_datapoint::Restriction::new()
                .set_namespace("capture_id")
                .set_allow_list([capture.id.to_string()]),
        ];

        let datapoint = IndexDatapoint::new()
            .set_datapoint_id(datapoint_id)
            .set_feature_vector(feature_vector)
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

    fn make_datapoint_id(&self, capture_id: i32, illumination_id: i32) -> String {
        format!("capture:{}:illumination:{}", capture_id, illumination_id)
    }
}

#[async_trait::async_trait]
impl Indexer for GeminiV2SearchIndexer {
    async fn index_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<SearchIndexUpsertResult> {
        GeminiV2SearchIndexer::index_capture(self, capture).await
    }
}

pub(super) fn parse_embedding_values(response: &Value) -> anyhow::Result<Vec<f32>> {
    let arrays = [
        response.pointer("/embedding/values"),
        response.pointer("/embeddings/0/values"),
        response.pointer("/embeddings/0/value"),
        response.pointer("/values"),
    ];

    let Some(raw_values) = arrays.into_iter().flatten().find_map(Value::as_array) else {
        anyhow::bail!(
            "embedContent response missing embedding values field: {}",
            response
        );
    };

    let mut values = Vec::with_capacity(raw_values.len());
    for (idx, value) in raw_values.iter().enumerate() {
        let Some(f) = value.as_f64() else {
            anyhow::bail!("Embedding value at index {} is not numeric", idx);
        };
        values.push(f as f32);
    }

    if values.is_empty() {
        anyhow::bail!("Embedding response returned an empty vector");
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_embedding_values_supports_embeddings_array_shape() {
        let value = json!({
            "embeddings": [
                {
                    "values": [0.1, 0.2, 0.3]
                }
            ]
        });

        let parsed = parse_embedding_values(&value).expect("should parse values");
        assert_eq!(parsed.len(), 3);
    }
}
