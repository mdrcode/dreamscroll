//! The identity vocabulary: who asserts a key, the key itself, and the assertion.
//!
//! This is knowledge2's own. knowledge1's [`Identity`](crate::knowledge::evidence::Identity)
//! deliberately leaves the provider to the writer, so a resolver cannot attribute its answer to
//! another provider. v2's caller *is* the policy, so the assertion carries its provider. They are
//! not the same type, and the seam between them is primitive strings.

use serde::{Deserialize, Serialize};

use crate::model;

use super::ids::ObservationId;

/// Who asserted an identity: `illuminator`, `wikidata`, `model`, `user`, `operator`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Provider(pub String);

/// The durable name of an identity: `(scheme, external_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityKey {
    pub scheme: String,
    pub external_id: String,
}

/// One provider's assertion of one identity about one entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: Provider,
    pub scheme: String,
    pub external_id: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub classes: Vec<String>,
    /// The observation that carried it, when a capture showed it.
    #[serde(default)]
    pub observation_id: Option<ObservationId>,
}

impl Identity {
    /// The platform link an observation carried, as an identity, or nothing.
    ///
    /// The caller's admission rule: a platform and a handle are enough. No handle, no identity.
    pub fn from_observation(observation: &model::user_observed_entity::Model) -> Option<Self> {
        let platform = observation.platform.as_deref()?;
        let handle = observation.platform_handle.as_deref()?;
        Some(Self {
            provider: Provider("illuminator".into()),
            scheme: crate::knowledge::text::comparable(platform),
            external_id: crate::knowledge::text::comparable(handle),
            method: Some("observation".into()),
            confidence: None,
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: Some(ObservationId(observation.id)),
        })
    }
}
