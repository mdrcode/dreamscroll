//! The identity vocabulary: who asserts a key, the key itself, and the assertion.
//!
//! The caller is the policy, so the assertion carries its provider. The identity key is the stable
//! pair `(scheme, external_id)`; metadata belongs to the provider that answered.

use serde::{Deserialize, Serialize};

use crate::model;

use super::ids::ObservationId;

/// Who asserted an identity: `illuminator`, `wikidata`, `model`, `user`, `operator`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Provider(pub String);

/// Provider ids that name our own pipeline, not an authority.
pub const RESERVED_PROVIDER_IDS: &[&str] = &["illuminator", "platform", "model", "operator", "user"];

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
            scheme: crate::knowledge2::text::comparable(platform),
            external_id: crate::knowledge2::text::comparable(handle),
            method: Some("observation".into()),
            confidence: None,
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: Some(ObservationId(observation.id)),
        })
    }
}
