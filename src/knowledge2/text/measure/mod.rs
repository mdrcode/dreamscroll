//! Comparison machinery for pluggable planners and resolvers.
//!
//! None of this is needed by the graph's integrity: the blocker index does not use tokens, grams, or
//! spelling tolerance. It exists for components that decide whether two surfaces mean the same thing.

mod surface;
mod tokens;

pub(crate) use surface::{near_surface, word_prefix};
pub use tokens::jaccard;
pub(crate) use tokens::{content_tokens, grams};
