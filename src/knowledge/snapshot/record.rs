//! A frozen copy of the projection, and where it lives.
//!
//! The suffix is the moment it was taken, so two snapshots sort chronologically and a run can be
//! named out loud: `knowledge_entities__20261102T181839351`.
//!
//! Snapshotting **copies** rather than renames. Renaming the live tables would drag the foreign key
//! from `user_observed_entities.knowledge_entity_id` along with them — Postgres keeps a constraint
//! pointing at the same table — so the new empty entity table would have nothing referencing it while
//! the archive did, and the fixed index name on the evidence table would then collide when it was
//! recreated. `CREATE TABLE AS SELECT` sidesteps all of it: a frozen artifact needs no constraints,
//! and the live schema is left exactly as it was.

use chrono::Utc;

pub(super) const CORE_TABLE: &str = "knowledge_entities";
pub(super) const EVIDENCE_TABLE: &str = "knowledge_evidence";
pub(super) const CLAIMS_TABLE: &str = "knowledge_claims";

/// A frozen copy of one run, named by the moment it was taken so two runs sort chronologically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub suffix: String,
}

impl Snapshot {
    pub fn now() -> Self {
        Self {
            suffix: Utc::now().format("%Y%m%dT%H%M%S%3f").to_string(),
        }
    }

    pub fn entities(&self) -> String {
        format!("{CORE_TABLE}__{}", self.suffix)
    }

    pub fn evidence(&self) -> String {
        format!("{EVIDENCE_TABLE}__{}", self.suffix)
    }

    pub fn claims(&self) -> String {
        format!("{CLAIMS_TABLE}__{}", self.suffix)
    }

    /// Which observations belonged to which entity — the only record of that is
    /// `user_observed_entities.knowledge_entity_id`, so snapshotting the entities alone would keep
    /// the referents and lose the membership. One row per linked observation.
    pub fn membership(&self) -> String {
        format!("knowledge_entity_membership__{}", self.suffix)
    }
}
