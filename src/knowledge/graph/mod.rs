//! The graph: what it holds, and every way it changes.
//!
//! [`KnowledgeGraph`] is the facade: every write to `knowledge_entities`, and every move of an
//! observation onto one, goes through its methods. Its parts:
//!
//! - `knowledge_graph` — the facade itself, and the primitives every write shares.
//! - `project` — the derived columns, including the blocking index row.
//! - [`merge`] — fold one entity into another.
//!
//! Single writes are methods; the batch passes are public functions, because they are commands over
//! the whole graph rather than a write to one row. A pass takes what it acts on as arguments — the
//! connection, and the [`Scoring`](crate::knowledge::scoring::Scoring) it measures with — rather than
//! reading them off a facade. The proximity passes live in [`crate::knowledge::proximity`]; the log
//! and its commands are the graph's verdict store, and a merge writes its row through it on the
//! fold's transaction.

mod link;
pub mod merge;

mod knowledge_graph;
mod project;

pub(crate) use knowledge_graph::{current_generation, get};
pub use knowledge_graph::KnowledgeGraph;
pub use link::LinkReport;
