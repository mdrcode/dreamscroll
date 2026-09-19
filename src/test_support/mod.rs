//! Test support utilities.
//!
//! Only compiled under `cfg(test)`. See `plan/testing.md` for the
//! project-wide testing philosophy.
//!
//! Submodules:
//!
//! - [`test_config`] — shared test configuration.
//! - [`test_db`] — the schema-isolated database harness for DB tests.

pub mod test_config;
pub mod test_db;
