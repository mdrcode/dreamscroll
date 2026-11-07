//! The vocabulary, shared by every layer: what a request is, what an answer is, what a claim is,
//! and the two traits that separate strategy from wire.
//!
//! Everything here is plain data or a trait. Nothing in this file knows about the cache, the budget,
//! or any particular authority.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::cache::Outcome;

/// How many queries go into one transport call. Providers that cannot batch ignore the chunking and
/// answer one request per query; the engine does not care.
#[cfg(not(test))]
pub(crate) const BATCH: usize = 50;

/// Smaller under test, so a second transport call — where the rate limit is exercised — is reachable
/// with a handful of entities.
#[cfg(test)]
pub(crate) const BATCH: usize = 2;

/// What a request asks. The ladder is this enum's order, and every dispatch on it is exhaustive —
/// a typo can no longer fall through to the label path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Exact label, through a SPARQL `VALUES`.
    Label,
    /// A Wikipedia article title, which is unique where a label is not.
    Title,
    /// MediaWiki full-text search: the expensive fallback.
    Search,
    /// A platform account, held by one Wikidata property (`P2002` for X, and so on).
    Handle(&'static str),
}

impl Kind {
    /// The name this kind answers to in the cache and in `shape`. Stored, so changing it invalidates
    /// answers — the same rule `shape` follows.
    pub fn to_key(&self) -> String {
        match self {
            Self::Label => "label".to_string(),
            Self::Title => "title".to_string(),
            Self::Search => "search".to_string(),
            Self::Handle(property) => format!("handle:{property}"),
        }
    }

    /// The property a handle request asks about, if it is one.
    pub fn property(&self) -> Option<&'static str> {
        match self {
            Self::Handle(property) => Some(property),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub provider: &'static str,
    pub kind: Kind,
    /// Hash of how the request is built; part of the cache key, so a changed query is a new question.
    pub shape: String,
    pub query: String,
}

/// One item an authority offered, as the authority described it.
///
/// Typed, because it used to make the round trip `Candidate -> json -> Value -> Candidate` between
/// every transport and every decision. `classes` is empty when the request did not ask for `P31`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub outcome: Outcome,
    /// What the authority answered with. Empty is a real answer: "nothing has that name".
    pub candidates: Vec<Candidate>,
}

/// Answers keyed by `(provider, kind, shape, query)`.
///
/// Every part earns its place: `title` and `label` ask about the same string and the answers are
/// different things; and two authorities can ask the *same* kind and query, so a map without the
/// provider would let one resolver overwrite another's answer before it was read.
pub type Answers = HashMap<(String, Kind, String, String), Raw>;

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
    fn batch_size(&self, _kind: &Kind) -> usize {
        BATCH
    }
}
