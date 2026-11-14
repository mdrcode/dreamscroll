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

/// Open the first line of every DB test: `db_or_skip!(db)` binds `db`, or returns when the machine
/// has no database so the test is skipped rather than half-run against nothing.
///
/// A statement, not an expression: the checked-out schema is dropped when `TestDb` is, and an
/// expression form would hand the schema back to the pool while the test was still using it.
macro_rules! db_or_skip {
    ($handle:ident) => {
        let Some(test_db) = $crate::test_support::test_db::test_db().await else {
            return;
        };
        let $handle = test_db.handle();
    };
}

pub(crate) use db_or_skip;

pub mod fixtures;
pub mod link;
pub mod test_config;
pub mod test_db;

/// A service client over the test database, for code that reaches the app the way a worker does.
pub fn service_api(db: &crate::database::DbHandle) -> crate::api::ServiceApiClient {
    let config = test_config::load().expect("test config should load");
    crate::api::ServiceApiClient::new(db.clone(), crate::storage::UrlMaker::from_config(&config))
}
