//! The graph: a facade, the SQL it runs, and the one derived write.

// `graph::graph` is the requested filename; the facade is one small file beside its parts.
#[allow(clippy::module_inception)]
mod graph;
mod project;
mod report;
mod selection;
mod sql;
mod validate;
mod verdicts;

pub use graph::{KnowledgeGraph, MergePreview};
pub(crate) use sql::generation as current_generation;
pub use project::Scope as ReprojectScope;
pub use report::{Refusal, Report};
pub use selection::GraphSelector;
pub use verdicts::VerdictSelector;
