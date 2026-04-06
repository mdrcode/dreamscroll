use anyhow;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{search, storage};

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub user_id: i32,
    pub created_at: DateTime<Utc>,
    pub created_at_human: String,
    pub medias: Vec<MediaInfo>,
    pub illuminations: Vec<IlluminationInfo>,
    pub annotation: Option<AnnotationInfo>,
}

impl search::DataObject for CaptureInfo {
    fn data_object_id(&self) -> String {
        format!("u{}-c{}", self.user_id, self.id)
    }

    fn data_object_json(&self) -> anyhow::Result<Map<String, Value>> {
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
}

#[derive(Clone)]
pub struct CaptureInfoEmbedPartsMaker {
    storage: Box<dyn storage::StorageProvider>,
}

impl CaptureInfoEmbedPartsMaker {
    pub fn new(storage: Box<dyn storage::StorageProvider>) -> Self {
        Self { storage }
    }
}

#[async_trait::async_trait]
impl search::gcloud::EmbedPartsMaker<CaptureInfo> for CaptureInfoEmbedPartsMaker {
    async fn make_embed_parts(&self, object: &CaptureInfo) -> anyhow::Result<serde_json::Value> {
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
