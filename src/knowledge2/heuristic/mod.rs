//! Automated, non-agentic linking and identifying.
//!
//! Everything here is a guess: the policy that ranks candidates, the measures it ranks with, and the
//! text comparisons those measures use. The graph's core does not depend on this module; an agentic
//! linker can replace it without moving the graph.
//!
//! - [`HeuristicLinker`] — a held identity wins; else the best qualifying candidate; else create.
//! - [`identification`] — the Wikidata-backed identity policy: plan, gather, rank, decide.
//! - [`scoring`] — every number the policy guesses.
//! - [`text`] — the non-core text measures (tokens, grams, Jaccard, near-surface, word-prefix).

pub mod identification;
pub mod identifier;
pub mod linker;
pub mod scoring;
pub mod text;

pub use identifier::HeuristicIdentifier;
pub use linker::HeuristicLinker;
pub use scoring::Heuristic;
