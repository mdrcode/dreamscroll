use serde::Serialize;

use crate::entity::*;

#[derive(Clone, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub provider: String,
    pub content: String,
}

impl From<illumination::ModelEx> for IlluminationInfo {
    fn from(mx: illumination::ModelEx) -> Self {
        Self {
            id: mx.id,
            capture_id: mx.capture_id,
            provider: mx.provider,
            content: mx.content,
        }
    }
}
