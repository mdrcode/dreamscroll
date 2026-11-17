//! The retrieval-key contract the graph owns.
//!
//! The blocker index stores these forms, the blocker probes with them, and projection uses the fold
//! to dedupe a member's surfaces. Nothing here scores or compares two texts.

mod fingerprint;
mod fold;
mod order;

pub(crate) use fingerprint::norm_fingerprint;
pub(crate) use fold::comparable;
pub(crate) use order::word_key;
