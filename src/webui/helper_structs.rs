use serde::Serialize;

use crate::entity::*;

// These are convenience structs for passing complex data to Tera templates.

#[derive(Serialize)]
pub struct CaptureInfo {
    pub capture: capture::Model,
    pub medias: Vec<media::Model>,
}
