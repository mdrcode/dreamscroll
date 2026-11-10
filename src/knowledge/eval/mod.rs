//! Offline diagnostics: why one entity resolved (or did not), and how well blocking recalls.
//!
//! Both tools call production decisions rather than restating them: `inspect` walks the same
//! blocker, `Scoring`, `plan` and `parse` the engine calls, and `blocker` replays observations in
//! capture order against the live index. The corpus-wide resolver replay was removed — it was a
//! second driver, and its selection/veto/cache rules drifted from the engine's.

pub mod blocker;
pub mod inspect;

pub use blocker::replay as replay_blocker;
pub use inspect::inspect;
