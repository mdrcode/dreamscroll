//! One item an authority offered, as the authority described it.
//!
//! Typed, because it used to make the round trip `WikidataCandidate -> json -> Value -> WikidataCandidate` between
//! every transport and every decision.
//!
//! This is the parsed shape every answer carries. A policy decides what the candidate becomes; the
//! conversion here is the authority's own identity shape, not a ranking.

use serde::{Deserialize, Serialize};

use crate::knowledge::identity::{Identity, Provider};

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
    /// The assertion a decided candidate becomes: this authority's id, the method that produced it,
    /// and the score behind the choice.
    pub(crate) fn identity(&self, method: &str, confidence: f32) -> Identity {
        Identity {
            provider: Provider(super::ID.to_string()),
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
