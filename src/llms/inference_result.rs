use serde_json::Value;

use super::InferenceMetadata;

pub trait InferenceResult: Send + Sync {
    fn raw_json(&self) -> &Value;
    fn metadata(&self) -> &InferenceMetadata;
}
