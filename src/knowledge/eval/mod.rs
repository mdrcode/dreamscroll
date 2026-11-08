//! Offline evaluation: replay a decision against answers already in the cache.
//!
//! The point is iteration speed. A resolver change normally costs a network run — 640 fetches for
//! the last corpus pass — and cannot be repeated at will. Replaying over `resolution_cache` answers
//! the same question in seconds, with no transport, so a gate can be changed and measured instead of
//! argued about.
//!
//! It is a *replay*, not a rehearsal: the decisions are the ones the resolver would make today given
//! the answers it has already seen. An entity whose answers are not cached is reported separately —
//! "could not be replayed" is not the same as "would not be claimed".

pub mod blocker;
pub mod inspect;
pub mod resolve;

pub use blocker::replay as replay_blocker;
pub use inspect::inspect;
pub use resolve::{Claimed, Report, replay};
