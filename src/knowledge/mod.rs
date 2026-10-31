//! The knowledge graph: observations, the cores they resolve to, and the evidence behind them.
//!
//! Design: `plan/knowledge-canonical-entities.md`. Nothing here calls an external service yet;
//! resolution lands next, and it will fill `core_knowledge_evidence`.

pub mod compare;
pub mod link;
pub mod migrate;
pub mod normalize;
pub mod reset;
pub mod score;
pub mod stats;
