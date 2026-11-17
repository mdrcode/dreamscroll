//! The mutations a caller may ask the graph to perform.
//!
//! One enum, no policy. A caller — the linker, an identity resolver, a person, a model — decides
//! what should happen and emits these values. [`KnowledgeGraph2`](crate::knowledge2::KnowledgeGraph2)
//! decides how to apply them and nothing else.

use serde::{Deserialize, Serialize};

use super::identity::{Identity, IdentityKey, Provider};
use super::ids::{EntityOriginId, ObservationId};

/// Why the mutation was made. Stored on the row; the lifting rule distinguishes the classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    Repair,
    Model,
    /// An older repair applied to this generation by replay.
    Replayed,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repair => "repair",
            Self::Model => "model",
            Self::Replayed => "replayed",
        }
    }
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
    },
    /// Lift a standing veto.
    UnvetoIdentity {
        subject: EntityOriginId,
        key: IdentityKey,
        cause: Cause,
    },
}
