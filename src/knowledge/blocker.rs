//! Blocking: which entities are worth considering as targets for linking an observation into the knowledge graph.
//!
//!
//! The one implementation is [`crate::knowledge::heuristic::NameBlocker`].

use crate::model;

/// The trait takes `&self` even where an implementation mutates. The store shares one blocker across
/// concurrent writers, and if that sharing were expressed here — `&mut self` behind a lock the store
/// owns — every caller would serialize on one lock, including the reads: `candidates` is called once
/// per observation and does the corpus-wide work. An implementation that holds an index therefore
/// owns its own synchronization and pays for the lock only where it writes. `Send + Sync` keeps that
/// honest: a `RefCell`-backed index will not compile.
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
    fn changed(&self, _entity: &model::knowledge_entity::Model) {}

    /// An entity that is gone — discarded, or tombstoned by a merge — and must stop being offered.
    ///
    /// The counterpart to [`Self::changed`]: an in-memory index that is only ever added to keeps
    /// handing back rows that no longer exist.
    fn forget(&self, _knowledge_entity_id: i32) {}
}
