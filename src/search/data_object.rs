use anyhow;
use serde_json::{Map, json};

use crate::api;

pub trait DataObject: Send + Sync {
    fn id(&self) -> String;
    fn object_data_json(&self) -> anyhow::Result<Map<String, serde_json::Value>>;
}

impl DataObject for api::CaptureInfo {
    fn id(&self) -> String {
        format!("u{}-c{}", self.user_id, self.id)
    }

    fn object_data_json(&self) -> anyhow::Result<Map<String, serde_json::Value>> {
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
