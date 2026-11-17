//! The graph's text contract: the fold, the keys derived from it, and their fingerprint.
//!
//! [`comparable`] is the folded surface, [`word_key`] the word-order-insensitive blocking key, and
//! [`norm_fingerprint`] the stamp on both. The blocker index stores them, the SQL blocker probes with
//! them, and projection dedupes a member's surfaces with the fold.
//!
//! Nothing here scores or compares two texts. The measures a planner or resolver ranks with live in
//! [`crate::knowledge2::heuristic::text`]; the graph never imports them.

mod fingerprint;
mod fold;
mod order;

pub(crate) use fingerprint::norm_fingerprint;
pub(crate) use fold::comparable;
pub(crate) use order::word_key;
