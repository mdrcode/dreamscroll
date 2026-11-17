//! Stable references: the two ids an action or an identity may carry.
//!
//! There is no entity id. An entity is named by the observation that created it, so both references
//! survive a rebuild.

use serde::{Deserialize, Serialize};

/// An observation id. Stable across every rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObservationId(pub i32);

/// The observation that created an entity: `knowledge_entities.origin_observation_id`.
///
/// The executor looks it up, follows tombstones to the live row, and never uses the observation's
/// current membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityOriginId(pub ObservationId);

impl From<i32> for ObservationId {
    fn from(id: i32) -> Self {
        Self(id)
    }
}

impl From<ObservationId> for i32 {
    fn from(id: ObservationId) -> Self {
        id.0
    }
}

impl From<ObservationId> for EntityOriginId {
    fn from(id: ObservationId) -> Self {
        Self(id)
    }
}

impl From<EntityOriginId> for ObservationId {
    fn from(id: EntityOriginId) -> Self {
        id.0
    }
}

impl From<EntityOriginId> for i32 {
    fn from(id: EntityOriginId) -> Self {
        id.0.0
    }
}
