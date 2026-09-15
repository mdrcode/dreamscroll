use std::str::FromStr;

use anyhow::{Context, bail};
use serde::{Deserialize, Deserializer};
use strum::{Display, EnumString};

use crate::{illumination, storage, task};

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

#[derive(Debug, Deserialize)]
pub struct Config {
    pub gcloud_project_id: String,
    pub gcloud_project_region: String,

    pub port: u16,

    #[serde(deserialize_with = "deserialize_comma_list")]
    pub services: Vec<Service>,

    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool, // true == only send cookies over HTTPS

    #[serde(default = "default_session_always_save")]
    pub session_always_save: bool, // true == refresh inactivity timeout on any valid request

    pub jwt_secret: Option<String>, // must be 32+ bytes for HS256 signing

    pub illuminator: String,
    pub gemini_api_key: Option<String>,
    pub gemini_model_id: Option<String>,
    #[serde(default = "default_gemini_payload_method")]
    pub gemini_payload_method: illumination::gemini::PayloadMethod,

    pub firestarter: String,
    pub xai_api_key: Option<String>,

    pub postgres_host_port: String, // e.g. "localhost:5432" or "db:5432"
    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_connection_params: Option<String>, // e.g. "sslmode=require"
    pub postgres_db: String,

    pub storage_backend: storage::StorageBackend,
    pub storage_local_file_path: Option<String>,
    pub storage_local_url_prefix: Option<String>,
    pub storage_gcloud_emulator: Option<String>, // e.g. "http://localhost:4443"
    pub storage_gcloud_prod_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: Option<String>,

    pub search_embed_collection_id: Option<String>,
    pub search_embed_vector_field: Option<String>,
    pub search_embed_vector_dims: Option<u32>,

    pub task_backend: task::TaskQueueBackend,
    pub task_cloudtask_queue_ingest: Option<String>,
    pub task_cloudtask_queue_illumination: Option<String>,
    pub task_cloudtask_queue_spark: Option<String>,
    pub task_cloudtask_queue_search_index: Option<String>,
}

pub fn make() -> anyhow::Result<Config> {
    let cfg = envy::from_env::<Config>()
        .context("Failed to load config (missing required env vars or invalid values)")?;

    match cfg.storage_backend {
        storage::StorageBackend::Local => {
            require_some(
                &cfg.storage_local_file_path,
                "STORAGE_BACKEND is local but no STORAGE_LOCAL_FILE_PATH",
            )?;
            require_some(
                &cfg.storage_local_url_prefix,
                "STORAGE_BACKEND is local but no STORAGE_LOCAL_URL_PREFIX",
            )?;
        }
        storage::StorageBackend::GCloud => {
            require_some(
                &cfg.storage_gcloud_bucket_name,
                "STORAGE_BACKEND is gcloud but no STORAGE_GCLOUD_BUCKET_NAME",
            )?;
        }
    }

    Ok(cfg)
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
