//! The vocabulary, shared by every layer: what a request is, what an answer is, what a claim is,
//! and the two traits that separate strategy from wire.
//!
//! Everything here is plain data or a trait. Nothing in this file knows about the cache, the budget,
//! or any particular authority.

use std::collections::HashMap;

use serde_json::Value;

use super::cache::Outcome;

/// How many queries go into one transport call. Providers that cannot batch ignore the chunking and
/// answer one request per query; the engine does not care.
#[cfg(not(test))]
pub(crate) const BATCH: usize = 50;

/// Smaller under test, so a second transport call — where the rate limit is exercised — is reachable
/// with a handful of entities.
#[cfg(test)]
pub(crate) const BATCH: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub provider: &'static str,
    pub kind: String,
    /// Hash of how the request is built; part of the cache key, so a changed query is a new question.
    pub shape: String,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub outcome: Outcome,
    pub payload: Option<Value>,
}

/// Answers keyed by `(provider, kind, shape, query)`.
///
/// Every part earns its place: `title` and `label` ask about the same string and the answers are
/// different things; and two authorities can ask the *same* kind and query, so a map without the
/// provider would let one resolver overwrite another's answer before it was read.
pub type Answers = HashMap<(String, String, String, String), Raw>;

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

/// What went wrong at the transport. The distinction matters: a `Failed` request is a fact about the
/// provider ("we could not reach it") and may be cached briefly, while a `RateLimited` response is an
/// instruction — stop, and come back after `Retry-After`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    RateLimited { retry_after_secs: u64 },
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::RateLimited { retry_after_secs } => {
                write!(f, "rate limited, retry after {retry_after_secs}s")
            }
            FetchError::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// The wire.
///
/// Separate from `Resolver` because the strategy is the resolver's business while the transport is
/// the narrow part the cache has to understand. A resolver that batches badly pays for it here.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// The only network call in the system. `requests` are all the same kind and shape.
    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError>;

    /// How many queries this kind can answer in one network call. The engine chunks by this and
    /// paces each call, so a kind that cannot batch is throttled per query instead of fired as a
    /// burst — which is how the search endpoint got us rate limited.
    fn batch_size(&self, _kind: &str) -> usize {
        BATCH
    }
}
