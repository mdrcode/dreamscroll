use anyhow;
use base64::Engine;
use serde_json::json;

use crate::{api, storage};

#[derive(Clone)]
pub struct CaptureInfoEmbedMaker {
    storage: Box<dyn storage::StorageProvider>,
}

impl CaptureInfoEmbedMaker {
    pub fn new(storage: Box<dyn storage::StorageProvider>) -> Self {
        Self { storage }
    }
}

impl CaptureInfoEmbedMaker {
    pub async fn make_embed_input(
        &self,
        object: &api::CaptureInfo,
    ) -> anyhow::Result<serde_json::Value> {
        let latest_illumination = object
            .illuminations
            .iter()
            .max_by_key(|illumination| illumination.id)
            .ok_or_else(|| {
                anyhow::anyhow!("Capture has no illumination, required for embedding")
            })?;
        let illumination_text = latest_illumination.make_text();

        let first_media = object
            .medias
            .first()
            .ok_or_else(|| anyhow::anyhow!("Capture has no media, required for embedding"))?;
        let image_bytes = self
            .storage
            .retrieve_bytes(&storage::StorageHandle::from(first_media))
            .await?;
        let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        tracing::debug!(
            capture_id = object.id,
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

        Ok(parts)
    }
}
