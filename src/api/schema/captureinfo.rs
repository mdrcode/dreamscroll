use chrono::{DateTime, Utc};

use serde::Serialize;

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct CaptureInfo {
    pub id: i32,
    pub user_id: i32,
    pub created_at: DateTime<Utc>,
    pub medias: Vec<MediaInfo>,
    pub illuminations: Vec<IlluminationInfo>,
    pub x_queries: Vec<String>,
    pub k_nodes: Vec<KNodeInfo>,
    pub social_medias: Vec<SocialMediaInfo>,
}


