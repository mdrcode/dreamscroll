use crate::{db, storage};

pub fn make_local_dev() -> (db::DbConfig, storage::StorageConfig) {
    let db_config = db::DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };

    let storage_config = storage::StorageConfig::Local {
        storage_path: "localdev/media/".to_string(),
        base_url: "/media/".to_string(),
    };

    (db_config, storage_config)
}
