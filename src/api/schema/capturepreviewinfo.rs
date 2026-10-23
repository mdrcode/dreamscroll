use serde::{Deserialize, Serialize};

/// JSON projection for a compact capture representation used by galleries and
/// spark previews. `thumbnail_url` is the image URL to render as a thumbnail;
/// it is not necessarily a separately generated thumbnail asset.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapturePreviewInfo {
    pub id: i32,
    pub thumbnail_url: String,
    pub summary: String,
}
