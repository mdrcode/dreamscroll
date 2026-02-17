use core::panic;

use serde::Deserialize;
use tracing;

use crate::{database, storage};

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,

    pub jwt_secret: Option<String>,

    pub illuminator_gemini_key: Option<String>,

    pub db_backend: database::DbBackend,
    pub db_url_sqlite: Option<String>,
    pub db_url_postgres: Option<String>,

    pub storage_backend: storage::StorageBackend,

    pub storage_local_file_path: Option<String>,
    pub storage_local_url_prefix: Option<String>,

    pub storage_gcloud_emulator_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: Option<String>,
}

fn default_port() -> u16 {
    8080
}

pub fn make_config() -> Config {
    let mut config = envy::prefixed("DREAMSCROLL_").from_env::<Config>().unwrap();

    // If $PORT is set, takes precedence over (prefixed) DREAMSCROLL_PORT for
    // environments like Google Cloud Run
    if std::env::var("PORT").is_ok() {
        config.port = std::env::var("PORT").unwrap().parse().unwrap();
        tracing::info!(
            "$PORT environment variable found, will listen on: {}",
            config.port
        );
    }

    match config.db_backend {
        database::DbBackend::Sqlite => {
            if config.db_url_sqlite.is_none() {
                panic!("DB_BACKEND is sqlite but no DREAMSCROLL_DB_URL_SQLITE");
            }
        }
        database::DbBackend::Postgres => {
            if config.db_url_postgres.is_none() {
                panic!("DB_BACKEND is postgres but no DREAMSCROLL_DB_URL_POSTGRES");
            }
        }
    }

    match config.storage_backend {
        storage::StorageBackend::Local => {
            if config.storage_local_file_path.is_none() {
                panic!("STORAGE_BACKEND is local but no DREAMSCROLL_STORAGE_LOCAL_FILE_PATH");
            }
            if config.storage_local_url_prefix.is_none() {
                panic!("STORAGE_BACKEND is local but no DREAMSCROLL_STORAGE_LOCAL_URL_PREFIX");
            }
        }
        storage::StorageBackend::GCloud => {
            if config.storage_gcloud_bucket_name.is_none() {
                panic!("STORAGE_BACKEND is gcloud but no DREAMSCROLL_STORAGE_GCLOUD_BUCKET_NAME");
            }
        }
    }

    config
}
