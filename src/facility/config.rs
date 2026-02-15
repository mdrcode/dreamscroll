use serde::{Deserialize, Deserializer};
use tracing;

use crate::{database, storage};

#[derive(Deserialize)]
pub struct Config {
    pub port: u16,

    #[serde(deserialize_with = "deserialize_tracing_level")]
    pub tracing_max_level: tracing::Level,

    pub db_backend: database::DbBackend,
    pub db_url_sqlite: String,
    pub db_url_postgres: Option<String>,

    pub jwt_secret: Option<String>,

    pub illuminator_gemini_key: Option<String>,

    pub storage_backend: storage::StorageBackend,

    pub storage_local_file_path: String,
    pub storage_local_url_prefix: String,

    pub storage_gcloud_emulator_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: String,
}

pub fn make_config() -> Config {
    let mut config = envy::prefixed("DREAMSCROLL_").from_env::<Config>().unwrap();

    // If $PORT is set, takes precedence over DREAMSCROLL_PORT for environments
    // like Google Cloud Run
    if std::env::var("PORT").is_ok() {
        config.port = std::env::var("PORT").unwrap().parse().unwrap();
    }

    config
}

fn deserialize_tracing_level<'de, D>(deserializer: D) -> Result<tracing::Level, D::Error>
where
    D: Deserializer<'de>,
{
    use std::str::FromStr;
    let s = String::deserialize(deserializer)?;
    tracing::Level::from_str(&s).map_err(serde::de::Error::custom)
}
