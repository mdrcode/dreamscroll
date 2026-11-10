//! The graph: what it holds, and every way it changes.
//!
//! [`KnowledgeGraph`] is the facade: every write to `knowledge_entities`, and every move of an
//! observation onto one, goes through its methods. Its parts:
//!
//! - `knowledge_graph` — the facade itself, and the primitives every write shares.
//! - `project` — the derived columns, including the blocking index row.
//! - [`merge`] — fold one entity into another.
//! - [`proximity`] — the candidates a scan found, and the pass that answers them.
//! - [`verdicts`] — a person's decisions, kept across rebuilds.
//!
//! Single writes are methods; the batch passes are public functions, because they are commands over
//! the whole graph rather than a write to one row. A pass that touches the pool takes it as an
//! argument alongside the facade ([`proximity::scan`], [`proximity::merge`]); the facade does not hand
//! its connection out.

mod link;
pub mod merge;
pub mod proximity;
pub mod verdicts;

mod knowledge_graph;
mod project;

pub(crate) use knowledge_graph::get;
pub use knowledge_graph::KnowledgeGraph;
pub use link::LinkStats;
pub use crate::model::knowledge_entity::Status;
