//! The graph: a facade, the SQL it runs, and the one derived write.

// `graph::graph` is the requested filename; the facade is one small file beside its parts.
#[allow(clippy::module_inception)]
mod graph;
mod project;
mod report;
mod selection;
mod sql;

pub use graph::KnowledgeGraph2;
pub use project::Scope as ReprojectScope;
pub use report::{Refusal, Report};
