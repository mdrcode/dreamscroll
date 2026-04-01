use anyhow;
use base64::Engine;
use serde_json::{Map, json};

use crate::{api, storage};

#[async_trait::async_trait]

pub trait DataObject: Send + Sync {
    fn data_object_id(&self) -> String;

    fn data_object_json(&self) -> anyhow::Result<Map<String, serde_json::Value>>;

    async fn parts_for_embed(
        &self,
        storage: &dyn storage::StorageProvider,
    ) -> anyhow::Result<serde_json::Value>;
}

#[async_trait::async_trait]
impl DataObject for api::CaptureInfo {
    fn data_object_id(&self) -> String {
        format!("u{}-c{}", self.user_id, self.id)
    }

    fn data_object_json(&self) -> anyhow::Result<Map<String, serde_json::Value>> {
        let latest_illumination = self
            .illuminations
            .iter()
            .max_by_key(|illumination| illumination.id)
            .ok_or_else(|| {
                anyhow::anyhow!("Capture has no illumination, required for embedding")
            })?;
        let illumination_text = latest_illumination.make_text();

        // note that ID fields are strings (matching schema_vertex_data.json)
        let data = json!({
            "user_id": self.user_id.to_string(),
            "capture_id": self.id.to_string(),
            "illumination_id": latest_illumination.id.to_string(),
            "illumination_text": illumination_text,
        })
        .as_object()
        .cloned()
        .expect("data_object json");

        Ok(data)
    }

    async fn parts_for_embed(
        &self,
        storage: &dyn storage::StorageProvider,
    ) -> anyhow::Result<serde_json::Value> {
        let latest_illumination = self
            .illuminations
            .iter()
            .max_by_key(|illumination| illumination.id)
            .ok_or_else(|| {
                anyhow::anyhow!("Capture has no illumination, required for embedding")
            })?;
        let illumination_text = latest_illumination.make_text();

        let first_media = self
            .medias
            .first()
            .ok_or_else(|| anyhow::anyhow!("Capture has no media, required for embedding"))?;
        let image_bytes = storage
            .retrieve_bytes(&storage::StorageHandle::from(first_media))
            .await?;
        let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        tracing::info!(
            capture_id = self.id,
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
