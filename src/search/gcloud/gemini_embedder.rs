use anyhow::Context;
use base64::Engine;
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde_json::{Value, json};

use crate::{api, facility, search, storage};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const MODEL_ID: &str = "gemini-embedding-2-preview";
const OUTPUT_DIMS: u32 = 768;

/// Embeds a capture into a dense vector via Gemini Embeddings v2.
#[derive(Clone)]
pub struct GeminiEmbedder {
    project_id: String,
    region: String,
    model_id: String,
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
        Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            MODEL_ID.to_string(),
            OUTPUT_DIMS,
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

        Ok(Self {
            project_id,
            region,
            model_id,
            output_dims,
            storage,
            http: reqwest::Client::new(),
            adc_credentials,
        })
    }

    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    async fn embed_capture_impl(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<search::CaptureEmbedding> {
        let media = capture
            .medias
            .first()
            .context("Capture has no media; cannot embed for search")?;

        let illumination = capture
            .illuminations
            .first()
            .context("Capture has no illumination; search embedding requires text context")?;

        let hybrid_doc = search::docmaker::make_hybrid_document(capture, media, illumination)?;
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

        let datapoint_id = make_datapoint_id(capture.id, illumination.id);

        tracing::info!(
            capture_id = capture.id,
            illumination_id = illumination.id,
            datapoint_id,
            dimensions = embedding.len(),
            "Search embedding generated"
        );

        Ok(search::CaptureEmbedding {
            user_id: capture.user_id,
            capture_id: capture.id,
            illumination_id: illumination.id,
            datapoint_id,
            embedding,
        })
    }

    async fn embed_hybrid_document(
        &self,
        doc: &search::HybridSearchDocument,
        image_bytes: &[u8],
    ) -> anyhow::Result<Vec<f32>> {
        let access_token = self.adc_credentials.access_token().await?.token;
        let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:embedContent",
            self.region, self.project_id, self.region, self.model_id
        );

        let body = json!({
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
            },
            "config": {
                "output_dimensionality": self.output_dims
            }
        });

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
        parse_gemini_v2_embedding_json(&value)
    }
}

#[async_trait::async_trait]
impl search::Embedder for GeminiEmbedder {
    async fn embed_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<search::CaptureEmbedding> {
        self.embed_capture_impl(capture).await
    }
}

fn make_datapoint_id(capture_id: i32, illumination_id: i32) -> String {
    format!("capture:{}:illumination:{}", capture_id, illumination_id)
}

pub(super) fn parse_gemini_v2_embedding_json(response: &Value) -> anyhow::Result<Vec<f32>> {
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
