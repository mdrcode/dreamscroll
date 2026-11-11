//! One claim about an entity's identity: what was said, how sure, and what was asked.
//!
//! The vocabulary every writer of `knowledge_claims` shares. A capture's observation and an
//! authority's answer are both claims; only [`ClaimBasis`] and who filled the other fields differ.
//!
//! [`ClaimBasis`] lives with the column it fills (`model::knowledge_claims`) and is re-exported
//! here, so the knowledge layer names the same type the model stores.

use super::identity::ILLUMINATOR;
use crate::model;

pub use crate::model::knowledge_claims::ClaimBasis;

/// One identity a provider is willing to claim for an entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub scheme: String,
    /// The identifier inside the scheme — `Q42`, `tt0133093`, `@handle`.
    pub external_id: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub method: String,
    pub confidence: f32,
    /// What the provider was asked about. The resolver knows, because it chose the question.
    pub basis: ClaimBasis,
}

/// An edge about to be written: one entity claims one identity.
///
/// The identity is not here: it is settled by [`super::store::IdentitySettlement`], and the resolved
/// id is supplied by the write operation.
pub struct ClaimDraft<'a> {
    pub basis: ClaimBasis,
    /// Who supplied this edge. The engine sets it from `resolver.id()`; a capture writes
    /// [`ILLUMINATOR`].
    pub provider: &'static str,
    /// How the edge was reached: the ladder step, or `capture`.
    pub method: String,
    /// The observation that carried it, when a capture did. `None` when an authority answered:
    /// nothing user-visible carried it, so there is no provenance to record.
    pub capture: Option<&'a model::user_observed_entity::Model>,
}

impl<'a> ClaimDraft<'a> {
    /// A capture's claim: the route is always the handle it showed.
    pub fn capture(observation: &'a model::user_observed_entity::Model) -> Self {
        Self {
            basis: ClaimBasis::Handle,
            provider: ILLUMINATOR,
            method: "capture".to_string(),
            capture: Some(observation),
        }
    }

    /// An authority's claim: the basis is the route the answer came by.
    pub fn authority(basis: ClaimBasis, provider: &'static str, method: String) -> Self {
        Self {
            basis,
            provider,
            method,
            capture: None,
        }
    }
}
