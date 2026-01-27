use serde::Serialize;

use crate::model;

#[derive(Clone, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub summary: String,
    pub details: String,
}

impl From<model::illumination::ModelEx> for IlluminationInfo {
    fn from(m: model::illumination::ModelEx) -> Self {
        Self {
            id: m.id,
            capture_id: m.capture_id,
            summary: m.summary,
            details: m.details,
        }
    }
}
