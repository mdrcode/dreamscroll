use crate::{database::DbConfig, storage::StorageConfig};

pub enum Env {
    LocalDev,
    Production,
}

#[derive(Clone)]
pub struct Config {
    pub db_config: DbConfig,
    pub storage_config: StorageConfig,
    pub webui_host_port: Option<String>,
}

impl Config {
    pub fn init_logging(&self) {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .init();
    }
}

pub fn make(env: Env) -> Config {
    match env {
        Env::LocalDev => {
            let db_config = DbConfig::SqliteFile {
                path: "localdev/dreamspot.db".to_string(),
            };
            let storage_config = StorageConfig::Local {
                storage_path: "localdev/media/".to_string(),
                base_url: "/media/".to_string(),
            };
            return Config {
                db_config,
                storage_config,
                webui_host_port: Some("127.0.0.1:8000".to_string()),
            };
        }

        Env::Production => {
            unimplemented!();
        }
    }
}

pub fn init_logging() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
}
