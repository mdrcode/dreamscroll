//! The blocker: which entities are worth considering as targets.
//!
//! One implementation, [`SqlBlocker`], answers from `knowledge_blocking_index`. The trait is the seam a second
//! one would use; the in-memory one that proved the SQL against a map has done its job and is gone.

pub mod index;
mod sql;

pub use sql::SqlBlocker;

use crate::model;

/// `&self`, not `&mut self`: one blocker is shared across writers, so an in-memory index owns its own
/// synchronization and pays for the lock only where it writes. `Send + Sync` keeps that honest.
#[async_trait::async_trait]
pub trait Blocker: Send + Sync {
    /// Candidates for one observation. Pure: same corpus, same answer.
    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> Vec<model::knowledge_entity::Model>;

    /// One entity the run created or extended mid-run, which later observations must be able to find.
    fn changed(&self, _entity: &model::knowledge_entity::Model) {}

    /// An entity that is gone — discarded, or tombstoned by a merge — and must stop being offered.
    fn forget(&self, _knowledge_entity_id: i32) {}
}
