use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapturePreviewInfo {
    pub id: i32,
    pub url: String,
    pub summary: String,
}
