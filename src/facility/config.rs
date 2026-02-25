use anyhow::{Context, bail};
use serde::Deserialize;

use crate::{database, pubsub, storage};

fn default_cookie_secure() -> bool {
    true
}

#[derive(Deserialize)]
pub struct Config {
    pub project_id: String, // for Gcloud services (logs, pubsub, storage)

    pub port: u16,

    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool, // true == only send cookies over HTTPS (production)

    pub jwt_secret: Option<String>, // must be 32+ bytes for HS256 signing

    pub gemini_api_key: Option<String>,

    pub db_backend: database::DbBackend,

    pub db_url_sqlite: Option<String>,

    #[serde(skip)]
    pub db_postgres: Option<database::PostgresConfig>,

    pub storage_backend: storage::StorageBackend,

    pub storage_local_file_path: Option<String>,
    pub storage_local_url_prefix: Option<String>,

    pub storage_gcloud_emulator_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: Option<String>,

    #[serde(skip)]
    pub pubsub: pubsub::gcloud::PubSubConfig,
}

pub fn make_config() -> anyhow::Result<Config> {
    let mut config = envy::from_env::<Config>()
        .context("Failed to load config (missing required vars or invalid values)")?;

    config.pubsub = envy::prefixed("PUBSUB_")
        .from_env::<pubsub::gcloud::PubSubConfig>()
        .context("Failed to load PUBSUB_ env config (need PUBSUB_API_BASE_URL, PUBSUB_PROJECT_ID, PUBSUB_ILLUMINATION_TOPIC_ID)")?;

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
