use tracing;

use crate::{database::DbConfig, storage::StorageConfig};

use super::Env;

pub struct Config {
    pub tracing_max_level: tracing::Level,
    pub db_config: DbConfig,
    pub storage_config: StorageConfig,
    pub web_host_port: Option<(String, u16)>,
}

pub fn make_config(env: Env) -> Config {
    match env {
        Env::LocalDev => {
            let db_config = DbConfig::SqliteFile {
                path: "localdev/dreamscroll.db".to_string(),
            };
            let storage_config = StorageConfig::Local {
                storage_path: "localdev/media/".to_string(),
                base_url: "/media/".to_string(),
            };
            return Config {
                tracing_max_level: tracing::Level::INFO,
                db_config,
                storage_config,
                web_host_port: Some(("0.0.0.0".to_string(), 8000)),
            };
        }

        Env::Production => {
            unimplemented!();
        }
    }
}
