use serde::Serialize;

use crate::model::*;

// These are convenience structs for passing complex data to Tera templates.

#[derive(Serialize)]
pub struct CaptureInfo {
    pub capture: capture::Model,
    pub medias: Vec<media::Model>,
}
