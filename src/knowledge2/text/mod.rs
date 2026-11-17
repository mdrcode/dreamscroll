//! Text, split by who needs it.
//!
//! **`key` is the graph's contract.** The blocker index stores the folded surface and the
//! word-order-insensitive key; the SQL blocker probes with the same two; `norm_fingerprint` stamps
//! which rule produced a row. Projection also uses the fold to dedupe a member's surfaces.
//!
//! **`measure` is for pluggable components.** Tokens, grams, Jaccard, near-surface and word-prefix
//! are how the heuristic planner and the identity resolvers decide whether two texts mean the same
//! thing. The graph's integrity does not depend on them: if both components became LLM-driven and did
//! their own text, `key` would remain and `measure` would go with them.
//!
//! Identity-key folding (`Identity::from_observation`, v1's `IdentityKey::try_from`) is
//! producer-side: the caller supplies canonical `(scheme, external_id)` strings.

pub mod key;
pub mod measure;

// Compatibility re-exports for v1's `knowledge::text` alias. The v2 core imports `text::key`
// explicitly; these keep v1 call sites unchanged until v1 is deleted.
pub(crate) use key::{comparable, word_key};
pub(crate) use measure::{content_tokens, grams, near_surface, word_prefix};
pub use measure::jaccard;
