use serde::{Deserialize, Serialize};

/// JSON projection for a compact capture representation, used by galleries: spark
/// previews and the entity page's related captures. `thumbnail_url` is the image
/// URL to render as a thumbnail; it is not necessarily a separately generated
/// thumbnail asset.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapturePreviewInfo {
    pub id: i32,
    pub thumbnail_url: String,
    pub summary: String,
    /// Humanized capture date — a preview fact, so a gallery need not fetch the capture again.
    #[serde(default)]
    pub created_at_human: String,
    /// The observation that connects this capture to the page being viewed, when the surface
    /// navigates by occurrence rather than by capture. Spark leaves it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<i32>,
}
