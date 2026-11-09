//! One assertion about an entity's identity: what was said, how sure, and what was asked.
//!
//! The vocabulary every writer of `knowledge_claims` shares. A capture's observation and an
//! authority's answer are both claims; only [`ClaimBasis`] and who filled the other fields differ.
//!
//! [`ClaimBasis`] lives with the column it fills (`model::knowledge_claims`) and is re-exported
//! here, so the knowledge layer names the same type the model stores.

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
