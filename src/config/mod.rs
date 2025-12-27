use crate::{database, storage};

pub enum Env {
    LocalDev,
    Production,
}

pub fn make(env: Env) -> (database::DbConfig, storage::StorageConfig) {
    match env {
        Env::LocalDev => {
            let db_config = database::DbConfig::SqliteFile {
                path: "localdev/dreamspot.db".to_string(),
            };
            let storage_config = storage::StorageConfig::Local {
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
