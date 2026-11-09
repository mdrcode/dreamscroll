//! The resolution contract: what a provider is willing to assert about an entity, and the trait it
//! implements to say so.
//!
//! Request, answer and transport shapes are deliberately *not* here — they belong to
//! [`crate::fetch`], which knows nothing about knowledge entities. This file knows nothing about the
//! network, the cache or the budget.

use crate::fetch::Gateway;
use crate::model;

/// One authority, and the strategy for asking it.
///
/// `resolve` takes a *batch* of entities rather than one: batching across entities is the difference
/// between a hundred transport calls for a corpus walk and five thousand, and only the resolver knows
/// how to group its own requests.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    fn id(&self) -> &'static str;

    /// Claims about the entities it could resolve, each tagged with the entity it belongs to, in
    /// priority order: the engine takes the first claim per entity that the graph accepts.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Claim)>;
}

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
