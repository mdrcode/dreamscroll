use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub id: i32,
    pub storage_id: String,
    pub url: String,

    #[serde(skip)]
    pub storage_provider: String,

    #[serde(skip)]
    pub storage_bucket: Option<String>,

    #[serde(skip)]
    pub storage_shard: Option<String>,
}
