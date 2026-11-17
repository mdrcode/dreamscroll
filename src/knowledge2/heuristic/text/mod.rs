//! Non-core text: the measures a planner or resolver ranks with.
//!
//! None of this is needed by the graph's integrity: the blocker index does not use tokens, grams, or
//! spelling tolerance. It exists for the heuristic here, and for any future policy that decides
//! whether two surfaces mean the same thing.

mod surface;
mod tokens;

pub(crate) use surface::{near_surface, word_prefix};
pub use tokens::jaccard;
pub(crate) use tokens::{content_tokens, grams};
