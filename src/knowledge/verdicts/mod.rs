//! The verdict store: every write to `knowledge_verdicts`, and the reads a decision needs.
//!
//! One table, two statements. A **merge** row says two things were one, keyed by each side's
//! `origin_observation_id` so it survives a rebuild; the store writes it, but on the merge's
//! transaction rather than its own, so the fold and its log entry commit together. A **veto** row
//! says an entity is not an identity: `vetoed` answers admission, and a replay applies it.
//!
//! The register is data, and the store knows nothing about entities beyond the origins a verdict
//! names. Applying it to a rebuilt graph is the graph's command (`graph/replay.rs`): a rebuild
//! renumbers entities, so replay asks what each origin names now, folds the pairs that still map,
//! and withdraws the vetoed pairings. Anything that no longer maps is reported and dropped: erosion
//! is the price of the key, and a silent wrong merge is not.
//!
//! `store` holds the type, the merge log's append and the reads the replay and `stats` share; `veto`
//! the standing decisions admission asks about; `origin` the readers that turn an origin into the
//! entity it names now.

mod origin;
mod store;
mod veto;

pub(crate) use store::VerdictStore;
pub(crate) use store::pair_key;

/// Why a decision was made — the column `knowledge_verdicts.cause` names. Re-exported here because
/// this module owns that table's vocabulary; `merge` takes one, `replay` selects on it.
pub use crate::model::knowledge_verdicts::Cause;

#[cfg(test)]
mod tests;
