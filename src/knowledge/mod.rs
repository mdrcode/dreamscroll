//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.

pub mod cache;
pub mod blocking;
pub mod compare;
pub mod current;
pub mod link;
pub mod linker;
pub mod merge;
pub mod normalize;
pub mod reset;
pub mod resolve;
pub mod score;
pub mod stats;
pub mod view;
