//! Test support utilities.
//!
//! Only compiled under `cfg(test)`. See `_project/plans/testing.md` for the
//! project-wide testing philosophy.
//!
//! Submodules:
//!
//! - [`db`] — the isolated-database harness for DB tests.

pub mod db;
