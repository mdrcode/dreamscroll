//! The mutations a caller may ask the graph to perform.
//!
//! One enum, no policy. A caller — the linker, an identity resolver, a person, a model — decides
//! what should happen and emits these values. [`KnowledgeGraph`](crate::knowledge::KnowledgeGraph)
//! decides how to apply them and nothing else.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use super::identity::{Identity, IdentityKey, Provider};
use super::ids::{EntityOriginId, ObservationId};

/// Why the mutation was made. Stored on the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumIter, DeriveActiveEnum)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Cause {
    #[sea_orm(string_value = "repair")]
    Repair,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repair => "repair",
        }
    }
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn record_verdict_default() -> bool {
    true
}

/// Everything a caller may ask of the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mutation", rename_all = "snake_case")]
pub enum GraphMutation {
    /// Make a new entity from one unlinked observation, optionally asserting one identity.
    CreateEntity {
        from: ObservationId,
        #[serde(default)]
        with: Option<Identity>,
    },
    /// Point an observation at an existing entity, optionally asserting one identity.
    AttachObservation {
        observation: ObservationId,
        entity: EntityOriginId,
        #[serde(default)]
        with: Option<Identity>,
        #[serde(default)]
        why: Option<String>,
    },
    /// Leave an observation unlinked. Does not touch any identity.
    DetachObservation {
        observation: ObservationId,
        #[serde(default)]
        why: Option<String>,
    },
    /// Fold the loser into the winner.
    MergeEntity {
        loser: EntityOriginId,
        winner: EntityOriginId,
        cause: Cause,
        why: String,
        /// Replay applies an existing decision without appending a copy. Never serialized.
        #[serde(skip, default = "record_verdict_default")]
        record_verdict: bool,
    },
    /// Record that two entities are deliberately separate.
    MarkDistinct {
        a: EntityOriginId,
        b: EntityOriginId,
        cause: Cause,
        why: String,
    },
    /// Assert an identity about an entity.
    Identify {
        subject: EntityOriginId,
        identity: Identity,
    },
    /// Remove one provider's assertion. No standing decision is recorded.
    RetractIdentity {
        subject: EntityOriginId,
        provider: Provider,
        key: IdentityKey,
    },
    /// Record that an identity is not evidence for an entity, and delete it.
    VetoIdentity {
        subject: EntityOriginId,
        key: IdentityKey,
        cause: Cause,
        why: String,
        /// Replay applies an existing decision without appending a copy. Never serialized.
        #[serde(skip, default = "record_verdict_default")]
        record_verdict: bool,
    },
    /// Lift a standing veto.
    UnvetoIdentity {
        subject: EntityOriginId,
        key: IdentityKey,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_verdict_is_not_serialized_and_defaults_true() {
        let mutation = GraphMutation::MergeEntity {
            loser: EntityOriginId(ObservationId(1)),
            winner: EntityOriginId(ObservationId(2)),
            cause: Cause::Repair,
            why: "test".into(),
            record_verdict: false,
        };
        let json = serde_json::to_value(&mutation).unwrap();
        assert!(json.get("record_verdict").is_none());

        let parsed: GraphMutation = serde_json::from_value(json).unwrap();
        match parsed {
            GraphMutation::MergeEntity { record_verdict, .. } => assert!(record_verdict),
            other => panic!("expected a merge, got {other:?}"),
        }
    }
}
