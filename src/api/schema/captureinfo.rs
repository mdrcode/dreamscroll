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
    pub illuminations: Vec<IlluminationInfo>,
    pub annotation: Option<AnnotationInfo>,
}
