use crate::{database, storage};

pub fn make_local_dev() -> (database::DbConfig, storage::StorageConfig) {
    let db_config = database::DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };

    let storage_config = storage::StorageConfig::Local {
        storage_path: "localdev/media/".to_string(),
        base_url: "/media/".to_string(),
    };

    (db_config, storage_config)
}
