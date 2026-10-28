use anyhow::Context;
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

use crate::{config, llms, search};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const MODEL_ID: &str = "gemini-embedding-2-preview";
const TASK_TYPE_RETRIEVAL_DOCUMENT: &str = "RETRIEVAL_DOCUMENT";
const TASK_TYPE_RETRIEVAL_QUERY: &str = "RETRIEVAL_QUERY";

/// Embeds into a dense vector via Gemini Embeddings v2 of dimensionality `dims`.
#[derive(Clone)]
pub struct GeminiEmbedder {
    model_url: String,
    model_id: String,
    dims: u32,
    http: Client,
    adc_credentials: AccessTokenCredentials,
}

impl GeminiEmbedder {
    pub fn from_config(cfg: &config::Config) -> anyhow::Result<Self> {
        let output_dims = cfg
            .search_embed_vector_dims
            .context("SEARCH_EMBED_VECTOR_DIMS required for search indexing")?;

        Self::new(
            cfg.gcloud_project_id.clone(),
            cfg.gcloud_region.clone(),
            MODEL_ID.to_string(),
            output_dims,
        )
    }

    pub fn new(
        project_id: String,
        region: String,
        model_id: String,
        output_dims: u32,
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
            model_id,
            dims: output_dims,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60)) // sanity
                .build()?,
            adc_credentials,
        })
    }

    async fn embed_normalizing(
        &self,
        parts: Value,
        task_type: &str,
    ) -> anyhow::Result<llms::InferenceOutcome<search::Embedding<f32, search::Unit>>> {
        let access_token = self.adc_credentials.access_token().await?.token;

        let body = json!({
            "content": {
                "parts": parts
            },
            "embedContentConfig": {
                "outputDimensionality": self.dims,
                "taskType": task_type
            }
        });

        let started = std::time::Instant::now();
        let response = self
            .http
            .post(&self.model_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;
        let duration_ms = started.elapsed().as_millis() as i64;
        let json: Value =
            serde_json::from_str(&text).context("Invalid embedContent response JSON")?;
        Ok(embedding_response(
            json,
            status,
            self.model_id.clone(),
            duration_ms,
            self.dims,
            task_type,
        ))
    }
}

#[async_trait::async_trait]
impl search::Embedder<serde_json::Value, search::Embedding<f32, search::Unit>> for GeminiEmbedder {
    #[tracing::instrument(skip(self, query), fields(query_len = query.len()))]
    async fn embed_query(
        &self,
        query: &str,
    ) -> anyhow::Result<search::Embedding<f32, search::Unit>> {
        if query.trim().is_empty() {
            anyhow::bail!("Query text is empty, cannot embed");
        }

        let json_value = json!([
            {
                "text": query
            }
        ]);

        let embedding = self
            .embed_normalizing(json_value, TASK_TYPE_RETRIEVAL_QUERY)
            .await?
            .output?;
        tracing::debug!(dims = embedding.len(), "Query embedding generated");
        Ok(embedding)
    }

    #[tracing::instrument(skip(self, object))]
    async fn embed_object(
        &self,
        object: serde_json::Value,
    ) -> anyhow::Result<llms::InferenceOutcome<search::Embedding<f32, search::Unit>>> {
        let embedding = self
            .embed_normalizing(object, TASK_TYPE_RETRIEVAL_DOCUMENT)
            .await?;
        tracing::debug!("Object embedding response received");
        Ok(embedding)
    }
}

fn embedding_response(
    mut json: Value,
    status: reqwest::StatusCode,
    model_id: String,
    duration_ms: i64,
    dims: u32,
    task_type: &str,
) -> llms::InferenceOutcome<search::Embedding<f32, search::Unit>> {
    let mut metadata = llms::InferenceMetadata {
        provider_name: "gemini".to_owned(),
        backend_name: "vertex".to_owned(),
        model_id,
        duration_ms,
        tokens_in: json
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(Value::as_i64),
        ..Default::default()
    };
    let output = if status.is_success() {
        parse_gemini_v2_embedding_json(&json).and_then(search::Embedding::from_vec_normalizing)
    } else {
        Err(anyhow::anyhow!(
            "Gemini embedContent failed with status {status} and body: {json}"
        ))
    };
    if let Some(usage) = json.get_mut("usageMetadata").map(Value::take) {
        metadata
            .provider_metadata
            .insert("provider_usage".to_owned(), usage);
    }
    metadata
        .params
        .insert("embedding_task_type".to_owned(), json!(task_type));
    metadata
        .params
        .insert("output_dimensions".to_owned(), json!(dims));
    llms::InferenceOutcome {
        output,
        metadata,
        http_status_code: Some(status.as_u16()),
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
    fn embedding_usage_survives_invalid_vectors_and_preserves_large_input_counts() {
        let usage = json!({"promptTokenCount": 5_000_000_000_i64, "totalTokenCount": 8});
        let outcome = embedding_response(
            json!({"embedding": {"values": []}, "usageMetadata": usage}),
            reqwest::StatusCode::OK,
            "test-model".to_owned(),
            12,
            3,
            TASK_TYPE_RETRIEVAL_DOCUMENT,
        );
        assert!(outcome.output.is_err());
        assert_eq!(outcome.metadata.tokens_in, Some(5_000_000_000));
        assert_eq!(outcome.metadata.tokens_in_cached, None);
        assert_eq!(outcome.metadata.tokens_out, None);
        assert_eq!(outcome.metadata.tokens_thought, None);
        assert_eq!(outcome.metadata.provider_metadata["provider_usage"], usage);
        assert_eq!(
            outcome.metadata.params,
            json!({
                "embedding_task_type": TASK_TYPE_RETRIEVAL_DOCUMENT,
                "output_dimensions": 3,
            })
            .as_object()
            .unwrap()
            .clone()
        );
    }

    #[test]
    fn missing_usage_does_not_invent_embedding_token_counts() {
        let outcome = embedding_response(
            json!({"embeddings": [{"values": [3.0, 4.0]}]}),
            reqwest::StatusCode::OK,
            "test-model".to_owned(),
            12,
            2,
            TASK_TYPE_RETRIEVAL_DOCUMENT,
        );
        let vector = outcome.output.unwrap();
        assert!((vector.as_slice()[0] - 0.6).abs() < 0.00001);
        assert!((vector.as_slice()[1] - 0.8).abs() < 0.00001);
        assert_eq!(outcome.metadata.tokens_in, None);
        assert_eq!(outcome.metadata.tokens_out, None);
        assert_eq!(outcome.metadata.tokens_thought, None);
    }
}
