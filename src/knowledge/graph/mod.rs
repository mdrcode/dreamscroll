//! The graph: what it holds, and every way it changes.
//!
//! [`KnowledgeGraph`] is the facade: every write to `knowledge_entities`, and every move of an
//! observation onto one, goes through its methods. Its parts:
//!
//! - `knowledge_graph` — the facade itself, and the primitives every write shares.
//! - `project` — the derived columns, including the blocking index row.
//! - [`merge`] — fold one entity into another.
//!
//! Single writes are methods. Passes over *derived* data are public functions taking what they act
//! on — the connection, and the [`Scoring`](crate::knowledge::scoring::Scoring) they measure with —
//! because they are not part of the graph's contract: [`crate::knowledge::proximity`] lives above
//! and is driven by a caller. Operations over the graph's own state are methods, because the graph
//! is the only handle a caller has on them: `link`, `merge`, and replay, which lives here because
//! applying the log is a move on the graph while the log itself is the verdict store.

mod link;
pub mod merge;

mod knowledge_graph;
mod project;
mod replay;

pub use knowledge_graph::KnowledgeGraph;
pub(crate) use knowledge_graph::{current_generation, get};
pub use link::LinkReport;
pub use project::Scope as ReprojectScope;
pub use replay::ReplayReport;
