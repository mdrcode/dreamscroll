//! One item an authority offered, as the authority described it.
//!
//! Typed, because it used to make the round trip `WikidataCandidate -> json -> Value -> WikidataCandidate` between
//! every transport and every decision.
//!
//! This is the parsed shape every answer carries. A policy decides what the candidate becomes; the
//! core does not know about graph identities.

use serde::{Deserialize, Serialize};

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
