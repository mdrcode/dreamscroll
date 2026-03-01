use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MediaInfo {
    pub id: i32,

    pub storage_uuid: Uuid,
    pub url: String,

    pub mime_type: Option<String>,

    #[serde(skip)]
    pub hash_blake3: Option<String>,

    #[serde(skip)]
    pub storage_provider: String,

    #[serde(skip)]
    pub storage_bucket: Option<String>,

    #[serde(skip)]
    pub storage_shard: String,

    #[serde(skip)]
    pub storage_extension: Option<String>,
}
