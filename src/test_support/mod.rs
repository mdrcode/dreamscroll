//! Test support utilities.
//!
//! Only compiled under `cfg(test)`. See `plan/testing.md` for the
//! project-wide testing philosophy.
//!
//! Submodules:
//!
//! - [`test_config`] — shared test configuration.
//! - [`test_db`] — the schema-isolated database harness for DB tests.
//! - [`fixtures`] — model builders for a user/capture/illumination/entity graph.

pub mod fixtures;
pub mod link;
pub mod test_config;
pub mod test_db;

/// A service client over the test database, for code that reaches the app the way a worker does.
pub fn service_api(db: &crate::database::DbHandle) -> crate::api::ServiceApiClient {
    let config = test_config::load().expect("test config should load");
    crate::api::ServiceApiClient::new(db.clone(), crate::storage::UrlMaker::from_config(&config))
}
