use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub id: i32,
    pub storage_id: String,
    pub url: String,
}
