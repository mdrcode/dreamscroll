use serde::{Deserialize, Serialize};

use crate::model;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SocialMediaInfo {
    pub id: i32,
    pub display_name: String,
    pub handle: String,
    pub platform: String,
}

impl From<model::social_media::ModelEx> for SocialMediaInfo {
    fn from(m: model::social_media::ModelEx) -> Self {
        Self {
            id: m.id,
            display_name: m.display_name,
            handle: m.handle,
            platform: m.platform,
        }
    }
}
