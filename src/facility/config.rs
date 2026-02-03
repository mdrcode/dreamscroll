use tracing;

use crate::{auth, database, storage};

use super::Env;

pub struct Config {
    pub environment: Env,
    pub tracing_max_level: tracing::Level,
    pub db_config: database::DbConfig,
    pub storage_config: storage::StorageConfig,
    pub jwt_config: auth::JwtConfig,
    pub web_host_port: Option<(String, u16)>,
}

pub fn make_config(env: Env) -> Config {
    match env {
        Env::LocalDev => {
            let db_config = database::DbConfig::SqliteFile {
                path: "localdev/dreamscroll.db".to_string(),
            };
            let local_storage_config = storage::LocalConfig {
                storage_path: "localdev/local_storage_provider".to_string(),
                web_path: "/media/".to_string(),
            };
            let gcloud_emulator_storage = storage::GCloudConfig {
                emulator_endpoint: Some("http://localhost:4443".to_string()),
                bucket: "dreamscroll-test1".to_string(),
            };
            let jwt_config = std::env::var("JWT_SECRET")
                .map(|secret| auth::JwtConfig::from_secret(secret.as_bytes()))
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "JWT_SECRET not set, using default localdev secret. \
                     This won't work in production!"
                    );
                    auth::JwtConfig::from_secret(
                        b"dreamscroll-local-dev-secret-key-not-for-production",
                    )
                });
            return Config {
                environment: Env::LocalDev,
                tracing_max_level: tracing::Level::INFO,
                db_config,
                storage_config: storage::StorageConfig::GCloud(gcloud_emulator_storage),
                jwt_config,
                web_host_port: Some(("0.0.0.0".to_string(), 8000)),
            };
        }

        Env::Production => {
            unimplemented!();
        }
    }
}
