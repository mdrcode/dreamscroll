//! Offline diagnostics: how well blocking recalls.
//!
//! `blocker` replays observations in capture order against the live index. It calls the production
//! blocker rather than restating it. The per-entity resolver trace lives with the resolver, in
//! `identity::inspect`; the corpus-wide resolver replay was removed because it was a second driver
//! whose selection, veto and cache rules drifted from the engine's.

pub mod blocker;

pub use blocker::replay as replay_blocker;
