//! Automated, non-agentic planning and reconciling.
//!
//! Everything here is a guess: the policy that ranks candidates, the measures it ranks with, and the
//! text comparisons those measures use. The graph's core does not depend on this module; an agentic
//! planner can replace it without moving the graph.
//!
//! - [`HeuristicPlanner`] — a held identity wins; else the best qualifying candidate; else create.
//! - [`resolution`] — the Wikidata-backed identity policy: plan, gather, rank, decide.
//! - [`scoring`] — every number the policy guesses.
//! - [`text`] — the non-core text measures (tokens, grams, Jaccard, near-surface, word-prefix).

pub mod planner;
pub mod resolution;
pub mod scoring;
pub mod text;

pub use planner::HeuristicPlanner;
pub use scoring::Heuristic;
