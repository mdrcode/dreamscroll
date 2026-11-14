//! One item an authority offered, as the authority described it.
//!
//! Typed, because it used to make the round trip `WikidataCandidate -> json -> Value -> WikidataCandidate` between
//! every transport and every decision.
//!
//! A decided candidate becomes the stored [`Identity`] with no second pass: the annotation *is* the
//! item, its method and its confidence, so the conversion lives on the type that carries them.

use serde::{Deserialize, Serialize};

use crate::knowledge::evidence::Identity;

/// `classes` is empty when the request did not ask for `P31`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikidataCandidate {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub classes: Vec<String>,
}

impl WikidataCandidate {
    /// The annotation a decided candidate becomes: this authority's id, the strategy that produced
    /// it, and the score behind that choice.
    pub(crate) fn identity(&self, method: &str, confidence: f32) -> Identity {
        Identity {
            scheme: super::ID.to_string(),
            external_id: self.id.clone(),
            method: Some(method.to_string()),
            confidence: Some(confidence),
            label: self.label.clone(),
            description: self.description.clone(),
            classes: self.classes.clone(),
            // A provider answer is not a capture: no observation carried it.
            observation_id: None,
        }
    }
}
