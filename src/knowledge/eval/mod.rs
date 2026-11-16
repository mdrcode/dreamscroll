//! Offline diagnostics: how well blocking recalls, and how one entity reached its identity.
//!
//! `blocker` replays observations in capture order against the live index. `inspect` walks one
//! entity through blocking, ranking and the resolver's plan. Both call the production code rather
//! than restating it; the corpus-wide resolver replay was removed because it was a second driver
//! whose selection, veto and cache rules drifted from the engine's.

pub mod blocker;
pub mod inspect;

pub use blocker::replay as replay_blocker;
