//! The resolution contract: what a provider is willing to assert about an entity.
//!
//! Request, answer and transport shapes are deliberately *not* here — they belong to
//! [`crate::fetch`], which knows nothing about knowledge entities. This file knows nothing about the
//! network, the cache or the budget.

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
    pub basis: crate::knowledge::evidence::ClaimBasis,
}
