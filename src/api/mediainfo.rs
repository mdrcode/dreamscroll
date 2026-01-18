use serde::Serialize;

use crate::entity::*;

#[derive(Clone, Serialize)]
pub struct MediaInfo {
    pub id: i32,
    pub capture_id: Option<i32>,
    pub filename: String,
}

impl From<media::ModelEx> for MediaInfo {
    fn from(mx: media::ModelEx) -> Self {
        Self {
            id: mx.id,
            capture_id: mx.capture_id,
            filename: mx.filename,
        }
    }
}
