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

use crate::knowledge::view;

#[async_trait::async_trait]
pub trait Blocker: Send + Sync {
    /// Generates knowledge entity link candidates for one observation.
    async fn candidates(&self, observation: &view::ObservationView) -> Vec<i32>;

    /// Notifies the blocker that an entity has changed (e.g. newly created or updated).
    fn changed(&mut self, _entity: &view::EntityView) {}
}
