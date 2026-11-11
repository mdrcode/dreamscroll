//! One claim about an entity's identity: who holds it, what identity, and how it was reached.
//!
//! The vocabulary every writer of `knowledge_claims` shares. An observation and a provider answer
//! are both claims; what differs is the provenance values, not the shape.

use super::PLATFORM_LINK_PROVIDER_ID;
use super::identity::IdentityKey;
use crate::model;

/// One entity's claim to one identity.
///
/// The identity's metadata lives in [`super::identity::IdentityDraft`]; this is only the edge. A
/// provider may propose several claims for one entity, each with its own method.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub holder: i32,
    pub identity: IdentityKey,
    /// Who supplied this edge. The engine sets it from `resolver.id()`; an observation writes
    /// [`PLATFORM_LINK_PROVIDER_ID`].
    pub provider: &'static str,
    /// How the edge was reached: the ladder step, or `observation`.
    pub method: String,
    /// The observation that carried it; `None` for a provider answer.
    pub observation_id: Option<i32>,
}

impl Claim {
    /// A claim carried by an observation: the handle it showed is the route.
    pub fn from_observation(
        holder: i32,
        observation: &model::user_observed_entity::Model,
        identity: IdentityKey,
    ) -> Self {
        Self {
            holder,
            identity,
            provider: PLATFORM_LINK_PROVIDER_ID,
            method: "observation".to_string(),
            observation_id: Some(observation.id),
        }
    }

    /// A claim from a provider answer.
    pub fn from_provider(
        holder: i32,
        identity: IdentityKey,
        provider: &'static str,
        method: String,
    ) -> Self {
        Self {
            holder,
            identity,
            provider,
            method,
            observation_id: None,
        }
    }
}
