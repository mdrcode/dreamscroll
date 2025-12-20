use crate::storage::{self, StorageConfig};

#[derive(Clone, Copy)]
pub enum Environment {
    LocalDev,
    Production,
    UnitTest,
}

pub fn make_facility(env: Environment) -> Box<dyn Facility> {
    match env {
        Environment::LocalDev => Box::new(LocalDevFacility {}),
        Environment::Production => unimplemented!("Production facility not implemented"),
        Environment::UnitTest => unimplemented!("UnitTest facility not implemented"),
    }
}

pub trait Facility: Send + Sync {
    fn db_config(&self) -> crate::db::DbConfig;
    fn storage_config(&self) -> StorageConfig;
    fn ui_host_port(&self) -> String;
    fn clone_box(&self) -> Box<dyn Facility>;
}

impl Clone for Box<dyn Facility> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Clone)]
struct LocalDevFacility;

impl Facility for LocalDevFacility {
    fn db_config(&self) -> crate::db::DbConfig {
        crate::db::DbConfig::SqliteFile {
            path: "localdev/dreamspot.db".to_string(),
        }
    }

    fn storage_config(&self) -> storage::StorageConfig {
        // ensure the local storage directory exists
        std::fs::create_dir_all("localdev/media/").unwrap();

        StorageConfig::Local {
            config: storage::local::LocalStorageConfig {
                storage_path: "localdev/media/".to_string(),
                base_url: "http://localhost:8000/media/".to_string(),
            },
        }
    }

    fn ui_host_port(&self) -> String {
        "127.0.0.1:8000".to_string()
    }

    fn clone_box(&self) -> Box<dyn Facility> {
        Box::new(self.clone())
    }
}
