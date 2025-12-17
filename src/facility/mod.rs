#[derive(Clone, Copy)]
pub enum Environment {
    UnitTest,
    LocalDev,
    Production,
}

pub fn make_facility(env: Environment) -> Box<dyn Facility> {
    match env {
        Environment::UnitTest => unimplemented!("UnitTest facility not implemented"),
        Environment::LocalDev => Box::new(LocalDevFacility {}),
        Environment::Production => unimplemented!("Production facility not implemented"),
    }
}

pub trait Facility: Send + Sync {
    fn db_config(&self) -> crate::db::DbConfig;
    fn ui_host_port(&self) -> String;
    fn local_media_path(&self) -> String; // TODO should be factored into a StorageConfig ...
    fn clone_box(&self) -> Box<dyn Facility>;
}

impl Clone for Box<dyn Facility> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Clone)]
pub struct LocalDevFacility;

impl Facility for LocalDevFacility {
    fn db_config(&self) -> crate::db::DbConfig {
        crate::db::DbConfig::SqliteFile {
            path: "localdev/dreamspot.db".to_string(),
        }
    }

    fn ui_host_port(&self) -> String {
        "127.0.0.1:8000".to_string()
    }

    fn local_media_path(&self) -> String {
        "localdev/uploads".to_string()
    }

    fn clone_box(&self) -> Box<dyn Facility> {
        Box::new(self.clone())
    }
}
