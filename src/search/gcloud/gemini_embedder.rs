use anyhow::Context;
use base64::Engine;
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

use crate::{api, facility, search, storage};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const MODEL_ID: &str = "gemini-embedding-2-preview";
const TASK_TYPE_RETRIEVAL_DOCUMENT: &str = "RETRIEVAL_DOCUMENT";
const TASK_TYPE_RETRIEVAL_QUERY: &str = "RETRIEVAL_QUERY";

/// Embeds a capture into a dense vector via Gemini Embeddings v2.
#[derive(Clone)]
pub struct GeminiEmbedder {
    model_url: String,
    output_dims: u32,
    storage: Box<dyn storage::StorageProvider>,
    http: Client,
    adc_credentials: AccessTokenCredentials,
}

impl GeminiEmbedder {
    pub fn from_config(
        config: &facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> anyhow::Result<Self> {
        let output_dims = config
            .search_embed_vector_dims
            .context("SEARCH_EMBED_OUTPUT_DIMS required for search indexing")?;

        Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            MODEL_ID.to_string(),
            output_dims,
            storage,
        )
    }

    pub fn new(
        project_id: String,
        region: String,
        model_id: String,
        output_dims: u32,
        storage: Box<dyn storage::StorageProvider>,
    ) -> anyhow::Result<Self> {
        let adc_credentials = Builder::default()
            .with_scopes([CLOUD_PLATFORM_SCOPE])
            .build_access_token_credentials()?;

        let model_url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:embedContent",
            region, project_id, region, model_id
        );

        tracing::info!(model_url, "GeminiEmbedder initialized");

        Ok(Self {
            model_url,
            output_dims,
            storage,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60)) // sanity
                .build()?,
            adc_credentials,
        })
    }

    async fn embed_content_parts_normalizing(
        &self,
        parts: Value,
        task_type: &str,
    ) -> anyhow::Result<search::Embedding<f32, search::Unit>> {
        let access_token = self.adc_credentials.access_token().await?.token;

        let body = json!({
            "content": {
                "parts": parts
            },
            "embedContentConfig": {
                "outputDimensionality": self.output_dims,
                "taskType": task_type
            }
        });

        let response = self
            .http
            .post(&self.model_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let err = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Gemini embedContent failed with status {} and body: {}",
                status,
                err
            );
        }

        let json: serde_json::Value = response.json().await?;
        let raw = parse_gemini_v2_embedding_json(&json)?;
        search::Embedding::from_vec_normalizing(raw)
    }
}

#[async_trait::async_trait]
impl search::Embedder<search::Embedding<f32, search::Unit>> for GeminiEmbedder {
    #[tracing::instrument(skip(self, query), fields(query_len = query.len()))]
    async fn embed_query(
        &self,
        query: &str,
    ) -> anyhow::Result<search::Embedding<f32, search::Unit>> {
        if query.trim().is_empty() {
            anyhow::bail!("Query text is empty, cannot embed");
        }

        let parts = json!([
            {
                "text": query
            }
        ]);

        let embed_normal = self
            .embed_content_parts_normalizing(parts, TASK_TYPE_RETRIEVAL_QUERY)
            .await?;
        tracing::info!(
            embedding_dims = embed_normal.len(),
            "Query embedding generated"
        );
        Ok(embed_normal)
    }

    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    async fn embed_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<search::CaptureEmbedding<search::Embedding<f32, search::Unit>>> {
        let latest_illumination = capture
            .illuminations
            .iter()
            .max_by_key(|illumination| illumination.id)
            .ok_or_else(|| {
                anyhow::anyhow!("Capture has no illumination, required for embedding")
            })?;
        let illumination_text = latest_illumination.make_text();

        let first_media = capture
            .medias
            .first()
            .ok_or_else(|| anyhow::anyhow!("Capture has no media, required for embedding"))?;
        let image_bytes = self
            .storage
            .retrieve_bytes(&storage::StorageHandle::from(first_media))
            .await?;
        let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        tracing::info!(
            capture_id = capture.id,
            illumination_id = latest_illumination.id,
            text_len = illumination_text.len(),
            image_b64_bytes = image_b64.len(),
            "Prepared embedding request"
        );

        let parts = json!([
            {
                "text": illumination_text
            },
            {
                "inlineData": {
                    "mimeType": first_media
                        .mime_type
                        .clone()
                        .unwrap_or_else(|| "image/jpeg".to_string()),
                    "data": image_b64
                }
            }
        ]);

        let embed_normal = self
            .embed_content_parts_normalizing(parts, TASK_TYPE_RETRIEVAL_DOCUMENT)
            .await?;
        let embed = search::CaptureEmbedding::<search::Embedding<f32, search::Unit>> {
            user_id: capture.user_id,
            capture_id: capture.id,
            illumination_id: latest_illumination.id,
            illumination_text: illumination_text,
            embedding: embed_normal,
        };

        tracing::info!(
            embedding = ?embed,
            "Capture embedding generated"
        );

        Ok(embed)
    }
}

fn parse_gemini_v2_embedding_json(response: &Value) -> anyhow::Result<Vec<f32>> {
    let pointers = [
        response.pointer("/embedding/values"),
        response.pointer("/embeddings/0/values"),
    ];

    let Some(found_embedding) = pointers.into_iter().flatten().find_map(Value::as_array) else {
        anyhow::bail!(
            "embedContent response missing embedding values field: {}",
            response
        );
    };

    let mut typed_embedding = Vec::with_capacity(found_embedding.len());
    for (idx, value) in found_embedding.iter().enumerate() {
        let Some(f) = value.as_f64() else {
            anyhow::bail!("Embedding value at index {} is not numeric", idx);
        };
        typed_embedding.push(f as f32);
    }

    if typed_embedding.is_empty() {
        anyhow::bail!("Embedding response returned an empty vector");
    }

    Ok(typed_embedding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_gemini_v2_embedding_json_supports_embeddings_array_shape() {
        let value = json!({
            "embeddings": [
                {
                    "values": [0.1, 0.2, 0.3]
                }
            ]
        });

        let parsed = parse_gemini_v2_embedding_json(&value).expect("should parse values");
        assert_eq!(parsed.len(), 3);
    }
}
