use google_cloud_aiplatform_v1::client::PredictionService;
use google_cloud_aiplatform_v1::model::{
    Blob, Content, FileData, GenerationConfig, Part, Tool, tool,
};
use serde::{Deserialize, Serialize};

use crate::{api, illumination, storage};

use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PayloadMethod {
    FileUri,
    Inline,
}

/// Gemini-based illumination powered by the Vertex AI API. Uses Application Default Credentials
/// and can pass either a GCS file URI or inline image bytes.
#[derive(Clone)]
pub struct GeminiVertexApiIlluminator {
    model_full_path: String,
    payload_method: PayloadMethod,
    storage: Box<dyn storage::StorageProvider>,
}

impl GeminiVertexApiIlluminator {
    pub fn new(
        project_id: &str,
        model_id: &str,
        payload_method: PayloadMethod,
        storage: Box<dyn storage::StorageProvider>,
    ) -> Self {
        let model_full_path = format!(
            "projects/{}/locations/global/publishers/google/models/{}",
            project_id, model_id
        );

        tracing::info!(model_full_path, "GeminiVertexApiIlluminator initialized");

        Self {
            model_full_path,
            payload_method,
            storage,
        }
    }
}

pub async fn make_media_payload(
    payload_method: PayloadMethod,
    media: &api::MediaInfo,
    storage: &dyn storage::StorageProvider,
) -> anyhow::Result<Part> {
    let storage_handle = storage::StorageHandle::from(media);
    let mime_type = media.mime_type.clone().unwrap_or("image/jpeg".to_string());

    match payload_method {
        PayloadMethod::FileUri => {
            let gcs_uri = storage
                .make_prod_uri(&storage_handle)
                .map_err(|e| anyhow::anyhow!("Failed to make GCS URI: {}", e))?;

            tracing::debug!(
                mime_type,
                storage_uuid = %storage_handle.uuid,
                gcs_uri,
                "Preparing file URI media payload for Illumination"
            );

            Ok(Part::new().set_file_data(
                FileData::new()
                    .set_mime_type(mime_type.clone())
                    .set_file_uri(gcs_uri),
            ))
        }
        PayloadMethod::Inline => {
            let storage_handle = storage::StorageHandle::from(media);
            let image_bytes = storage.retrieve_bytes(&storage_handle).await?;

            tracing::debug!(
                mime_type,
                storage_uuid = %storage_handle.uuid,
                image_bytes_len = image_bytes.len(),
                "Preparing inline data media payload for Illumination"
            );

            // Blob.data is raw bytes — gRPC/protobuf transport handles binary
            // natively; base64 encoding only needed for REST/JSON endpoints.
            Ok(Part::new().set_inline_data(
                Blob::new()
                    .set_mime_type(mime_type.clone())
                    .set_data(image_bytes),
            ))
        }
    }
}

#[async_trait::async_trait]
impl illumination::Illuminator for GeminiVertexApiIlluminator {
    fn name(&self) -> &'static str {
        "geminivertexapi"
    }

    /// Illuminates a capture and returns the structured response directly.
    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    async fn illuminate(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<illumination::Illumination> {
        let media1 = capture
            .medias.first()
            .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;

        let client = PredictionService::builder()
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create Vertex AI client: {}", e))?;

        let request_content = Content::new().set_role("user").set_parts(vec![
            Part::new().set_text(prompts::PROMPT.to_string()),
            make_media_payload(self.payload_method, media1, self.storage.as_ref()).await?,
        ]);

        let generation_config = GenerationConfig::new()
            .set_response_mime_type("application/json")
            .set_response_json_schema(response::make_response_schema());

        let tools = vec![Tool::new().set_google_search(tool::GoogleSearch::new())];

        tracing::info!(
            "Starting illumination of capture {} via Gemini Vertex",
            capture.id
        );
        let inference_start = std::time::Instant::now();
        let response = client
            .generate_content()
            .set_model(&self.model_full_path)
            .set_contents(vec![request_content])
            .set_generation_config(generation_config)
            .set_tools(tools)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Vertex AI API error: {}", e))?;
        let inference_duration = inference_start.elapsed();

        let first_candidate = response.candidates.first().ok_or_else(|| {
            anyhow::anyhow!(
                "Vertex AI did not return response candidate: {:?}",
                response
            )
        })?;

        let json_text = first_candidate
            .content
            .as_ref()
            .and_then(|content| content.parts.first())
            .and_then(|part| part.text())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Vertex AI did not return JSON text finish_reason: {:?}. Vertex AI response: {:?}",
                    first_candidate.finish_reason,
                    response
                )
            })?;

        let structured: response::GeminiStructuredResponse = serde_json::from_str(json_text)
            .inspect_err(|e| {
                tracing::warn!(
                    capture.id,
                    error = %e,
                    json_text = %json_text,
                    "Failed to parse GeminiStructuredResponse JSON from Vertex AI"
                );
            })?;

        tracing::info!(
            capture.id,
            illumination_vertexapi_ms = inference_duration.as_millis(),
            num_entities = ?structured.entities.len(),
            num_social_media_accounts = ?structured.social_media_accounts.len(),
            num_suggested_searches = ?structured.suggested_searches.len(),
            "GeminiVertexApiIlluminator: Success for capture {}",
            capture.id
        );

        Ok(illumination::Illumination::from(structured))
    }
}
