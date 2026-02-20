use core::panic;

use serde::Deserialize;

use crate::{database, storage};

#[derive(Deserialize)]
pub struct Config {
    pub port: u16,

    pub jwt_secret: Option<String>,

    pub illuminator_gemini_key: Option<String>,

    pub db_backend: database::DbBackend,
    pub db_url_sqlite: Option<String>,
    #[serde(skip)]
    pub db_postgres: Option<PostgresConfig>,

    pub storage_backend: storage::StorageBackend,

    pub storage_local_file_path: Option<String>,
    pub storage_local_url_prefix: Option<String>,

    pub storage_gcloud_emulator_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: Option<String>,

    #[serde(skip)]
    pub pubsub: PubSubConfig,
}

#[derive(Default, Deserialize)]
pub struct PostgresConfig {
    pub user: String,
    pub password: String,
    pub host_port: String, // e.g. "localhost:5432" or "db:5432"
    pub db: String,

    pub connection_params: Option<String>, // e.g. "sslmode=require"
}

#[derive(Default, Deserialize)]
pub struct PubSubConfig {
    pub emulator_url_base: Option<String>, // e.g. "http://localhost:8085"
    pub project_id: String,

    pub illumination_topic_id: String,

    pub push_oidc_audience: Option<String>,
    pub push_oidc_service_account_email: Option<String>,
    pub push_oidc_jwks_url: Option<String>,
}

pub fn make_config() -> Config {
    let mut config = envy::from_env::<Config>().unwrap();

    config.pubsub = envy::prefixed("PUBSUB_")
        .from_env::<PubSubConfig>()
        .expect("Failed to load PUBSUB_ env config (need PUBSUB_API_BASE_URL, PUBSUB_PROJECT_ID, PUBSUB_ILLUMINATION_TOPIC_ID)");

    if config.db_backend == database::DbBackend::Postgres {
        let postgres_config = envy::prefixed("POSTGRES_")
            .from_env::<PostgresConfig>()
            .expect("DB_BACKEND is postgres but missing full POSTGRES_ config (need POSTGRES_USER, POSTGRES_PASSWORD, POSTGRES_HOST, POSTGRES_DATABASE_NAME)");
        config.db_postgres = Some(postgres_config);
    }

    if config.db_backend == database::DbBackend::Sqlite {
        if config.db_url_sqlite.is_none() {
            panic!("DB_BACKEND is sqlite but no DB_URL_SQLITE");
        }
    }

    match config.storage_backend {
        storage::StorageBackend::Local => {
            if config.storage_local_file_path.is_none() {
                panic!("STORAGE_BACKEND is local but no STORAGE_LOCAL_FILE_PATH");
            }
            if config.storage_local_url_prefix.is_none() {
                panic!("STORAGE_BACKEND is local but no STORAGE_LOCAL_URL_PREFIX");
            }
        }
        storage::StorageBackend::GCloud => {
            if config.storage_gcloud_bucket_name.is_none() {
                panic!("STORAGE_BACKEND is gcloud but no STORAGE_GCLOUD_BUCKET_NAME");
            }
        }
    }

    config
}
