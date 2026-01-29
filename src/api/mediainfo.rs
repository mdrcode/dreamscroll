use serde::Serialize;

use crate::model;

#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub id: i32,
    pub capture_id: Option<i32>,
    pub filename: String,
}

impl From<model::media::ModelEx> for MediaInfo {
    fn from(mx: model::media::ModelEx) -> Self {
        Self {
            id: mx.id,
            capture_id: mx.capture_id,
            filename: mx.filename,
        }
    }
}
