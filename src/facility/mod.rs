use crate::{database::DbConfig, storage::StorageConfig};

pub enum Env {
    LocalDev,
    Production,
}

#[derive(Clone)]
pub struct Config {
    pub tracing_max_level: tracing::Level,
    pub db_config: DbConfig,
    pub storage_config: StorageConfig,
    pub webui_host_port: Option<String>,
}

pub fn make_config(env: Env) -> Config {
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
                tracing_max_level: tracing::Level::WARN,
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

pub fn init_logging(_config: &Config) {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
}
