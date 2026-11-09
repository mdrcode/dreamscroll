//! The graph: what it holds, and every way it changes.
//!
//! [`KnowledgeGraph`] is the facade and the only handle a caller gets — every write to
//! `knowledge_entities`, and every move of an observation onto one, goes through it. Its parts:
//!
//! - [`knowledge_graph`] — the facade itself, and the primitives every write shares.
//! - [`project`] — the derived columns, including the blocking index row.
//! - [`merge`] — fold one entity into another.
//! - [`proximity`] — the candidates a scan found, and the pass that answers them.
//! - [`verdicts`] — a person's decisions, kept across rebuilds.

pub mod merge;
pub mod proximity;
pub mod verdicts;

mod knowledge_graph;
mod project;

pub(crate) use knowledge_graph::get;
pub use knowledge_graph::KnowledgeGraph;
pub use crate::model::knowledge_entity::Status;
