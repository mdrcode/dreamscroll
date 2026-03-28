use std::str::FromStr;

use anyhow::{Context, bail};
use serde::{Deserialize, Deserializer};
use strum::{Display, EnumString};

use crate::{database, illumination, storage, task};

#[derive(Debug, Display, EnumString, PartialEq)]
#[strum(serialize_all = "lowercase")]
pub enum Service {
    WebUI,
    API,
    Webhook,
}

fn default_cookie_secure() -> bool {
    true
}

fn default_session_always_save() -> bool {
    true
}

fn default_gemini_payload_method() -> illumination::gemini::PayloadMethod {
    illumination::gemini::PayloadMethod::Inline
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(deserialize_with = "deserialize_comma_list")]
    pub services: Vec<Service>,

    pub gcloud_project_id: String,
    pub gcloud_project_region: String,

    pub port: u16,

    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool, // true == only send cookies over HTTPS

    #[serde(default = "default_session_always_save")]
    pub session_always_save: bool, // true == refresh inactivity timeout on any valid request

    pub jwt_secret: Option<String>, // must be 32+ bytes for HS256 signing

    pub illuminator: String,
    pub firestarter: String,
    pub gemini_api_key: Option<String>,
    #[serde(default = "default_gemini_payload_method")]
    pub gemini_payload_method: illumination::gemini::PayloadMethod,

    pub xai_api_key: Option<String>,

    pub db_backend: database::DbBackend,

    pub db_url_sqlite: Option<String>,

    #[serde(skip)]
    pub db_postgres: Option<database::PostgresConfig>,

    pub storage_backend: storage::StorageBackend,
    pub storage_local_file_path: Option<String>,
    pub storage_local_url_prefix: Option<String>,
    pub storage_gcloud_emulator: Option<String>, // e.g. "http://localhost:4443"
    pub storage_gcloud_prod_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: Option<String>,

    pub search_vertex_location: Option<String>,
    pub search_embedding_model_id: Option<String>,
    pub search_embedding_output_dimensionality: Option<u32>,
    pub search_vector_index_id: Option<String>,

    pub task_backend: task::TaskQueueBackend,
    pub task_pubsub_emulator: Option<String>, // e.g. "http://localhost:8085"
    pub task_pubsub_topic_new_capture: Option<String>,
    pub task_pubsub_topic_spark: Option<String>,
    pub task_cloudtask_queue_illumination: Option<String>,
    pub task_cloudtask_queue_spark: Option<String>,
}

pub fn make_config() -> anyhow::Result<Config> {
    let mut config = envy::from_env::<Config>()
        .context("Failed to load config (missing required env vars or invalid values)")?;

    if config.db_backend == database::DbBackend::Postgres {
        let postgres_config = envy::prefixed("POSTGRES_")
            .from_env::<database::PostgresConfig>()
            .context("DB_BACKEND is postgres but missing full POSTGRES_ config (need POSTGRES_USER, POSTGRES_PASSWORD, POSTGRES_HOST, POSTGRES_DATABASE_NAME)")?;
        config.db_postgres = Some(postgres_config);
    }

    if config.db_backend == database::DbBackend::Sqlite {
        require_some(
            &config.db_url_sqlite,
            "DB_BACKEND is sqlite but no DB_URL_SQLITE provided",
        )?;
    }

    match config.storage_backend {
        storage::StorageBackend::Local => {
            require_some(
                &config.storage_local_file_path,
                "STORAGE_BACKEND is local but no STORAGE_LOCAL_FILE_PATH",
            )?;
            require_some(
                &config.storage_local_url_prefix,
                "STORAGE_BACKEND is local but no STORAGE_LOCAL_URL_PREFIX",
            )?;
        }
        storage::StorageBackend::GCloud => {
            require_some(
                &config.storage_gcloud_bucket_name,
                "STORAGE_BACKEND is gcloud but no STORAGE_GCLOUD_BUCKET_NAME",
            )?;
        }
    }

    Ok(config)
}

fn require_some<T>(value: &Option<T>, message: &str) -> anyhow::Result<()> {
    if value.is_none() {
        bail!("{}", message);
    }
    Ok(())
}

fn deserialize_comma_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    String::deserialize(deserializer)?
        .split(',')
        .map(|item| item.trim().parse::<T>().map_err(serde::de::Error::custom))
        .collect()
}
