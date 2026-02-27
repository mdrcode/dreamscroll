use google_cloud_aiplatform_v1::client::PredictionService;
use google_cloud_aiplatform_v1::model::{Content, FileData, GenerationConfig, Part};

use crate::{api, illumination, storage};

use super::*;

/// Gemini-based illumination powered by the Vertex AI API. Uses Application Default Credentials
/// and passes a Google Storage URI instead of raw bytes.
#[derive(Clone)]
pub struct GeminiVertexApiIlluminator {
    model_path_full: String,
    storage: Box<dyn storage::StorageProvider>,
}

impl GeminiVertexApiIlluminator {
    pub fn new(project_id: &str, model: &str, storage: Box<dyn storage::StorageProvider>) -> Self {
        let model_path_full = format!(
            "projects/{}/locations/global/publishers/google/models/{}",
            project_id, model
        );

        tracing::info!(model_path_full, "GeminiVertexApiIlluminator initialized");

        Self {
            model_path_full,
            storage,
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
            .medias
            .get(0)
            .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;

        let storage_handle = storage::StorageHandle::from(media1);
        let gcs_uri = self
            .storage
            .make_prod_uri(&storage_handle)
            .map_err(|e| anyhow::anyhow!("Failed to make GCS URI: {}", e))?;

        tracing::info!(
            capture.id,
            media1.id,
            gcs_uri,
            "GeminiVertexApiIlluminator: preparing for illumination",
        );

        let client = PredictionService::builder()
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create Vertex AI client: {}", e))?;

        let request_content = Content::new().set_role("user").set_parts(vec![
            Part::new().set_text(prompts::PROMPT.to_string()),
            Part::new().set_file_data(
                FileData::new()
                    .set_mime_type(media1.mime_type.clone().unwrap_or("image/jpeg".to_string()))
                    .set_file_uri(gcs_uri),
            ),
        ]);

        let generation_config = GenerationConfig::new()
            .set_response_mime_type("application/json")
            .set_response_json_schema(response::make_response_schema());

        let response = client
            .generate_content()
            .set_model(&self.model_path_full)
            .set_contents(vec![request_content])
            .set_generation_config(generation_config)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Vertex AI API error: {}", e))?;

        let json_text = response
            .candidates
            .first()
            .and_then(|c| c.content.as_ref())
            .and_then(|content| content.parts.first())
            .and_then(|part| part.text())
            .ok_or_else(|| anyhow::anyhow!("No text content in Vertex AI response"))?;

        let structured: response::GeminiStructuredResponse = serde_json::from_str(json_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse structured response: {}", e))?;

        tracing::info!(
            capture.id,
            num_entities = ?structured.entities.len(),
            num_social_media_accounts = ?structured.social_media_accounts.len(),
            num_suggested_searches = ?structured.suggested_searches.len(),
            "GeminiVertexApiIlluminator: Successfully parsed illumination response",
        );

        Ok(illumination::Illumination::from(structured))
    }
}
