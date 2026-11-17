//! The blocker: which entities are worth considering as targets.
//!
//! One implementation, [`SqlBlocker`], answers from `knowledge_blocking_index`. The trait is the seam
//! a second one would use; the in-memory one that proved the SQL against a map has done its job and
//! is gone.

pub mod index;
mod sql;

pub use sql::SqlBlocker;

use crate::model;

/// `&self`, not `&mut self`: one blocker answers every link in a run, and the implementation owns
/// whatever state it keeps. `Send + Sync` keeps that honest.
#[async_trait::async_trait]
pub trait Blocker: Send + Sync {
    /// Candidates for one observation: live entities only, ordered by id.
    ///
    /// An error is not an empty set. Empty means "nothing matched, create"; a failed probe means
    /// "ask again", and conflating them silently forks the graph into duplicates.
    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>>;
}
