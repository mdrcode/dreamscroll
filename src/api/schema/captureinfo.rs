use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub user_id: i32,
    pub created_at: DateTime<Utc>,
    pub created_at_human: String,
    pub medias: Vec<MediaInfo>,
    /// At most **one** entry: the most recent illumination (highest `id`).
    ///
    /// Reruns append new illumination rows, so `InfoMaker` collapses them to the
    /// latest rather than exposing the history. Kept as a `Vec` for template and
    /// serde compatibility (templates use `| first`).
    pub illuminations: Vec<IlluminationInfo>,
    pub annotation: Option<AnnotationInfo>,
}
