use anyhow::Context;
use serde_json::json;

use crate::{api, config, illumination, llms, storage};

use super::{prompts, response};

#[derive(Clone)]
pub struct GeminiIlluminator {
    client: llms::gemini::GeminiInferenceClient,
    model_id: String,
    storage: Box<dyn storage::StorageProvider>,
}

impl GeminiIlluminator {
    pub fn new(
        cfg: &config::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> anyhow::Result<Self> {
        let model_id = cfg
            .gemini_model_id
            .as_deref()
            .context("GEMINI_MODEL_ID required when ILLUMINATOR=gemini")?
            .to_string();

        Ok(Self {
            client: llms::gemini::GeminiInferenceClient::from_config(cfg)?,
            model_id,
            storage,
        })
    }
}

#[async_trait::async_trait]
impl illumination::Illuminator for GeminiIlluminator {
    fn name(&self) -> &'static str {
        "gemini"
    }

    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    async fn illuminate(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<illumination::Illumination> {
        let media = capture
            .medias
            .first()
            .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;
        let storage_handle = storage::StorageHandle::from(media);
        let image = self.storage.retrieve_bytes(&storage_handle).await?;
        let mime_type = media.mime_type.as_deref().unwrap_or("image/jpeg");
        let input = [
            llms::gemini::GeminiInputPart::Text(prompts::PROMPT),
            llms::gemini::GeminiInputPart::InlineImage {
                bytes: image.as_ref(),
                mime_type,
            },
        ];
        let schema = response::make_response_schema();
        let tools = [json!({ "type": "google_search" })];

        tracing::info!(
            capture.id,
            media.id,
            image_bytes = image.len(),
            mime_type,
            "Starting Gemini Interactions illumination"
        );
        let inference_start = std::time::Instant::now();
        let interaction = self
            .client
            .interact(llms::gemini::GeminiInteractionRequest {
                model_id: &self.model_id,
                input: &input,
                response_schema: Some(&schema),
                tools: &tools,
                // Do not retain screenshot interactions server-side; Dreamscroll persists the
                // resulting illumination separately.
                store: false,
            })
            .await?;
        let structured_json = interaction.output_text()?;
        let structured: response::GeminiStructuredResponse = serde_json::from_str(&structured_json)
            .with_context(|| {
                format!(
                    "Failed to parse Gemini illumination JSON for capture {}",
                    capture.id
                )
            })?;

        tracing::info!(
            capture.id,
            interaction_id = ?interaction.id,
            num_entities = structured.entities.len(),
            num_social_media_accounts = structured.social_media_accounts.len(),
            num_suggested_searches = structured.suggested_searches.len(),
            gemini_interactions_ms = inference_start.elapsed().as_millis(),
            "Gemini Interactions illumination succeeded"
        );

        Ok(illumination::Illumination::from(structured))
    }
}
