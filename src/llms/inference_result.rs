use serde_json::Value;

use super::InferenceMetadata;

pub trait InferenceResult: Send + Sync {
    fn raw_json(&self) -> &Value;
    fn metadata(&self) -> &InferenceMetadata;

    /// Lossy Markdown projection for side-by-side evaluation and console display.
    fn to_markdown(&self) -> String;
}
