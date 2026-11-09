//! The vocabulary shared by every layer: what a request asks, what an answer is, and the trait that
//! separates a provider's strategy from the wire.
//!
//! Nothing here knows about the cache, the budget, any particular provider, or what an answer means.
//! A provider supplies its own request kinds and its own payload as associated types, so this module
//! can store, group and hand them back without ever matching on one of them.

use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;

use serde::{Serialize, de::DeserializeOwned};

/// How many queries go into one transport call. Providers that cannot batch override
/// [`Transport::batch_size`] and answer one request per query; the gateway does not care.
#[cfg(not(test))]
pub(crate) const BATCH: usize = 50;

/// Smaller under test, so a second transport call — where the rate limit is exercised — is reachable
/// with a handful of queries.
#[cfg(test)]
pub(crate) const BATCH: usize = 2;

/// What became of one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The provider answered **with a payload**. Not a decision: whether it holds what the caller
    /// wanted is the caller's decision, and it may reject the lot.
    Found,
    /// The provider answered with nothing — a cached negative.
    Empty,
    /// The provider did not answer.
    Failed,
    /// The provider answered, but not in a way worth trusting — a search that never reached its
    /// backend, a body carrying an error alongside a 200. The payload is still handed over, because
    /// it may hold something useful, but the answer expires quickly instead of standing for a week.
    Degraded,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Found => "found",
            Outcome::Empty => "empty",
            Outcome::Failed => "failed",
            Outcome::Degraded => "degraded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "found" => Some(Outcome::Found),
            "empty" => Some(Outcome::Empty),
            "failed" => Some(Outcome::Failed),
            "degraded" => Some(Outcome::Degraded),
            _ => None,
        }
    }

    /// Whether the answer is worth parsing. A degraded answer still carries a payload.
    pub fn answered(self) -> bool {
        matches!(self, Outcome::Found | Outcome::Degraded)
    }
}

/// One question for one provider.
///
/// `kind` is the provider's own vocabulary — whatever it needs to dispatch on, which this module
/// never interprets. `kind` and `shape` are both part of the cache key: a request whose construction
/// changes is a different question, so no stale answer can be served against it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Request<K> {
    /// Whose answer this is: the provider being asked. Never the caller — the gateway already knows
    /// which transport it was handed — and never an endpoint, which is `shape`'s business. A
    /// namespace for the cache and for credit, not a promise that the answer is true.
    pub provider: &'static str,
    pub kind: K,
    /// How the request is built: the provider's own fingerprint of endpoint, language, limits and
    /// properties.
    pub shape: String,
    pub query: String,
}

/// What a provider said about one request.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer<P> {
    pub outcome: Outcome,
    /// The payload, typed by the provider, so it makes no `json` round trip on the way in or out.
    /// `None` when the provider answered nothing at all; [`Outcome::answered`] says whether there is
    /// anything to read.
    pub body: Option<P>,
}

impl<P> Answer<P> {
    /// An answer that carried nothing — the shape a transport gives a request that failed.
    pub fn nothing(outcome: Outcome) -> Self {
        Self {
            outcome,
            body: None,
        }
    }
}

/// Every answer a run got, keyed by the question itself.
///
/// The whole request is the key, so two providers — or two kinds, or two shapes — can ask about the
/// same string and never overwrite each other's answer before it is read.
pub type Answers<K, P> = HashMap<Request<K>, Answer<P>>;

/// What went wrong at the transport. The distinction matters: a `Failed` request is a fact about the
/// provider ("we could not reach it") and may be cached briefly, while a `RateLimited` response is an
/// instruction — stop, and come back after `Retry-After`. `retry_after_secs` is `None` when the
/// provider refused without saying for how long; the refusal is the fact, the wait is the courtesy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    RateLimited { retry_after_secs: Option<u64> },
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::RateLimited {
                retry_after_secs: Some(secs),
            } => write!(f, "rate limited, retry after {secs}s"),
            FetchError::RateLimited {
                retry_after_secs: None,
            } => write!(f, "rate limited"),
            FetchError::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// The wire.
///
/// Separate from any strategy a caller owns, because the wire is the narrow part the cache has to
/// understand. A transport that batches badly pays for it here.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// The provider's own request kinds. Stored in the cache and used to group a batch; never
    /// inspected here.
    type Kind: Clone + Eq + Ord + Hash + Display + Send + Sync;

    /// What one answer carries, typed by the provider so the cache is the only place it becomes
    /// JSON.
    type Payload: Serialize + DeserializeOwned + Send + Sync;

    /// The only network call in the system. `requests` are all the same kind and shape.
    async fn fetch(
        &self,
        requests: &[Request<Self::Kind>],
    ) -> Result<Vec<(String, Answer<Self::Payload>)>, FetchError>;

    /// How many queries this kind can answer in one network call. The gateway chunks by this and
    /// paces each call, so a kind that cannot batch is throttled per query instead of fired as a
    /// burst — which is how a search endpoint once got us rate limited.
    fn batch_size(&self, _kind: &Self::Kind) -> usize {
        BATCH
    }
}
