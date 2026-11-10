//! One assertion about an entity's identity: what was said, how sure, and what was asked.
//!
//! The vocabulary every writer of `knowledge_claims` shares. A capture's observation and an
//! authority's answer are both claims; only [`ClaimBasis`] and who filled the other fields differ.
//!
//! [`ClaimBasis`] lives with the column it fills (`model::knowledge_claims`) and is re-exported
//! here, so the knowledge layer names the same type the model stores.

use super::identity::IdentityKey;
use crate::model;

pub use crate::model::knowledge_claims::ClaimBasis;

/// One identity a provider is willing to assert about an entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub scheme: String,
    pub id: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub method: String,
    pub confidence: f32,
    /// What the provider was asked about. The resolver knows, because it chose the question.
    pub basis: ClaimBasis,
}

/// An edge about to be written: one entity asserts one identity.
pub struct ClaimDraft<'a> {
    pub holder: i32,
    pub identity: &'a IdentityKey,
    pub identity_id: i32,
    pub basis: ClaimBasis,
    /// The observation that asserted it, when a capture did. `None` when an authority answered:
    /// nothing user-visible asserted it, so there is no provenance to record.
    pub capture: Option<&'a model::user_observed_entity::Model>,
}

impl<'a> ClaimDraft<'a> {
    /// A capture's claim: the route is always the handle it showed.
    pub fn capture(
        holder: i32,
        observation: &'a model::user_observed_entity::Model,
        identity: &'a IdentityKey,
        identity_id: i32,
    ) -> Self {
        Self {
            holder,
            identity,
            identity_id,
            basis: ClaimBasis::Handle,
            capture: Some(observation),
        }
    }

    /// An authority's claim: the basis is the route the answer came by.
    pub fn authority(
        holder: i32,
        identity: &'a IdentityKey,
        identity_id: i32,
        basis: ClaimBasis,
    ) -> Self {
        Self {
            holder,
            identity,
            identity_id,
            basis,
            capture: None,
        }
    }
}

/// What became of an assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// The edge was written.
    Written,
    /// An editor withdrew this claim.
    Vetoed,
    /// This entity already asserts the identity. A repeat is not a conflict: whether both holders are
    /// one thing is `proximity`'s question.
    AlreadyClaimed,
}
