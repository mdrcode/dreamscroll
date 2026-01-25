use serde::Serialize;

use crate::model;

#[derive(Clone, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub provider: String,
    pub content: String,
}

impl From<model::illumination::ModelEx> for IlluminationInfo {
    fn from(mx: model::illumination::ModelEx) -> Self {
        Self {
            id: mx.id,
            capture_id: mx.capture_id,
            provider: mx.provider_name,
            content: mx.raw_content,
        }
    }
}
