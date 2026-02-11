use serde::Serialize;

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub summary: String,
    pub details: String,
    pub x_queries: Vec<String>,
    pub k_nodes: Vec<KNodeInfo>,
    pub social_medias: Vec<SocialMediaInfo>,
}
