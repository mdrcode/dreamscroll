//! Blocking: which entities are worth considering for an observation.
//!
//! Recall-oriented and allowed to over-generate. A blocker never decides anything and never touches
//! the network, but it may touch the database — the intended implementation queries an index of
//! normalised keys, and the in-memory one is the cheap version for tests and offline runs.
//!
//! How a blocker gets its view of the corpus is its own business, and it is not part of this trait:
//! construction happens where the corpus is known (`link`'s `make_blocker`). A database-backed
//! blocker ignores the corpus entirely; an in-memory one builds its index from it in `build_entities`.
//!
//! The one implementation is [`crate::knowledge::heuristic::NameBlocker`].

use crate::model;

#[async_trait::async_trait]
pub trait Blocker: Send + Sync {
    /// Candidates for one observation. Pure: same corpus, same answer.
    ///
    /// Entities rather than ids: the linker scores what it is given, and making the caller resolve
    /// ids is what used to force the executor to keep a position map it had no other use for.
    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> Vec<model::knowledge_entity::Model>;

    /// One entity the run created or extended mid-run, which later observations must be able to find.
    ///
    /// A no-op for anything that reads the database directly: the row is written before this is
    /// called, so the blocker already sees it. An in-memory blocker has to be told.
    fn changed(&mut self, _entity: &model::knowledge_entity::Model) {}
}
