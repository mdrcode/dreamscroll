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

        let model_url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:embedContent",
            region, project_id, region, model_id
        );

        tracing::info!(model_url, "GeminiEmbedder initialized");

        Ok(Self {
            model_url,
            output_dims,
            storage,
            http: reqwest::Client::new(),
            adc_credentials,
        })
    }
}

#[async_trait::async_trait]
impl search::Embedder for GeminiEmbedder {
    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    async fn embed_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<search::CaptureEmbedding> {
        let access_token = self.adc_credentials.access_token().await?.token;
        let text_doc = search::docmaker::make_text_doc(capture)?;
        let (mime_type, image_b64) = make_image_base64(capture, self.storage.as_ref()).await?;

        let body = json!({
            "content": {
                "parts": [
                    {
                        "text": text_doc.text
                    },
                    {
                        "inline_data": {
                            "mime_type": mime_type,
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
            .post(&self.model_url)
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

        let json: serde_json::Value = response.json().await?;
        let embedding = parse_gemini_v2_embedding_json(&json)?;

        tracing::info!(
            capture_id = capture.id,
            illumination_id = text_doc.illumination_id,
            dimensions = embedding.len(),
            "Search embedding generated"
        );

        Ok(search::CaptureEmbedding {
            user_id: capture.user_id,
            capture_id: capture.id,
            illumination_id: text_doc.illumination_id,
            embedding,
        })
    }
}

async fn make_image_base64(
    capture: &api::CaptureInfo,
    storage: &dyn storage::StorageProvider,
) -> anyhow::Result<(String, String)> {
    let media = capture
        .medias
        .first()
        .ok_or_else(|| anyhow::anyhow!("Capture has no media; cannot embed for search"))?;

    let mime_type = media
        .mime_type
        .clone()
        .unwrap_or_else(|| "image/jpeg".to_string());

    let image_bytes = storage
        .retrieve_bytes(&storage::StorageHandle::from(media))
        .await?;
    let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

    Ok((mime_type, image_b64))
}

fn parse_gemini_v2_embedding_json(response: &Value) -> anyhow::Result<Vec<f32>> {
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
