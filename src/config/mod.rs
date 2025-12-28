use crate::{database::DbConfig, storage::StorageConfig};

pub enum Env {
    LocalDev,
    Production,
}

pub fn make(env: Env) -> (DbConfig, StorageConfig) {
    match env {
        Env::LocalDev => {
            let db_config = DbConfig::SqliteFile {
                path: "localdev/dreamspot.db".to_string(),
            };
            let storage_config = StorageConfig::Local {
                storage_path: "localdev/media/".to_string(),
                base_url: "/media/".to_string(),
            };
            return (db_config, storage_config);
        }

        Env::Production => {
            unimplemented!();
        }
    }
}
