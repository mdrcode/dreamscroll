//! The identity vocabulary: an identity's durable name, a platform link a capture showed, and the
//! draft an authority's answer becomes.
//!
//! The row itself lives in `model::knowledge_identity`; these are the shapes `Evidence` writes it
//! from, and the pair every durable reference uses.

use super::claim::Claim;
use crate::knowledge::normalize;
use crate::model;

/// An identity's durable name: the pair a rebuild can reproduce.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentityKey {
    pub scheme: String,
    pub external_id: String,
}

impl IdentityKey {
    /// The key a capture's platform handle names, both halves folded so two spellings agree.
    pub fn from_capture(observation: &model::user_observed_entity::Model) -> Option<Self> {
        let platform = observation.platform.as_deref()?;
        let handle = observation.platform_handle.as_deref()?;
        Some(Self {
            scheme: normalize::comparable(platform),
            external_id: normalize::comparable(handle),
        })
    }
}

/// An identity and the answer whoever gave it.
///
/// The authority's label, description and classes are properties of the identity, not of any one
/// claim: an album's article describes the album whether one entity claims it or five do.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftIdentity {
    pub scheme: String,
    pub external_id: String,
    /// Who asserted it: `platform` for a handle a capture showed, else the provider's own id.
    pub asserted_by: String,
    /// How it was answered, on the first answer only: `capture`, a Wikidata property (`P2002`, …) for
    /// a handle join, or the ladder step (`exact_label`, `search_ranked`, …).
    pub method: Option<String>,
    pub confidence: Option<f32>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
}

impl DraftIdentity {
    /// An identity known only from a capture: the handle, and nothing about it.
    pub fn from_capture(key: &IdentityKey) -> Self {
        Self {
            scheme: key.scheme.clone(),
            external_id: key.external_id.clone(),
            asserted_by: "platform".to_string(),
            method: Some("capture".to_string()),
            confidence: None,
            label: None,
            description: None,
            classes: Vec::new(),
        }
    }

    /// The answer an authority gave: it knows what the thing is, not which capture asked.
    pub fn from_claim(claim: Claim, asserted_by: &str) -> Self {
        Self {
            scheme: claim.scheme,
            external_id: claim.id,
            asserted_by: asserted_by.to_string(),
            method: Some(claim.method),
            confidence: Some(claim.confidence),
            label: claim.label,
            description: claim.description,
            classes: claim.classes,
        }
    }

    /// The durable name of the identity this draft describes.
    pub fn key(&self) -> IdentityKey {
        IdentityKey {
            scheme: self.scheme.clone(),
            external_id: self.external_id.clone(),
        }
    }
}
