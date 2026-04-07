use anyhow;
use serde_json::{Map, Value, json};

use crate::{api, search};

impl search::DataObject for api::CaptureInfo {
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
