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
            let storage_config = storage::StorageConfig::Local {
                storage_path: "localdev/media/".to_string(),
                base_url: "/media/".to_string(),
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
                storage_config,
                jwt_config,
                web_host_port: Some(("0.0.0.0".to_string(), 8000)),
            };
        }

        Env::Production => {
            unimplemented!();
        }
    }
}
