//! The resolver's only door to the world: cache first, then a paced, budgeted fetch.
//!
//! A resolver cannot fetch. It hands `resolve_requests` the questions it wants answered, and this
//! module decides what is already known, what the budget allows, and when the provider has asked us
//! to stop. That is what lets one own its own strategy — including several passes — without being
//! able to bypass the cache, the pace or the budget.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::cache::{Answer, Cache, Outcome, UNPRODUCTIVE_TTL};
use super::contract::{Answers, FetchError, Raw, Request, Transport};
use super::engine::{Options, Report};

/// No rate limit seen. Distinguishes "the provider did not ask us to wait" from "wait zero seconds".
const NOT_LIMITED: u64 = u64::MAX;

/// What a resolver is given.
///
/// It is the only door to the world: a resolver cannot fetch, only resolve requests. That is what
/// lets one own its own strategy — including several passes — without being able to bypass the
/// cache, the pace or the budget.
pub struct Gateway<'a> {
    cache: &'a Cache<'a>,
    pacer: tokio::sync::Mutex<Pacer>,
    remaining: tokio::sync::Mutex<usize>,
    offline: bool,
    planned: AtomicUsize,
    cache_hits: AtomicUsize,
    fetched: AtomicUsize,
    failed: AtomicUsize,
    budget_exhausted: AtomicBool,
    rate_limited_after: AtomicU64,
}

impl<'a> Gateway<'a> {
    pub(crate) fn new(cache: &'a Cache<'a>, options: &Options) -> Self {
        Self {
            cache,
            pacer: tokio::sync::Mutex::new(Pacer::new(options.min_interval)),
            remaining: tokio::sync::Mutex::new(options.max_requests),
            offline: options.offline,
            planned: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            fetched: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            budget_exhausted: AtomicBool::new(false),
            rate_limited_after: AtomicU64::new(NOT_LIMITED),
        }
    }

    /// Take one transport call from the budget; false when it is gone.
    async fn take_call(&self) -> bool {
        let mut remaining = self.remaining.lock().await;
        if *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        true
    }

    /// Whether the provider has told us to stop. Checked between resolvers, not inside one.
    pub fn rate_limited_after(&self) -> Option<u64> {
        let after = self.rate_limited_after.load(Ordering::Relaxed);
        (after != NOT_LIMITED).then_some(after)
    }

    pub fn budget_exhausted(&self) -> bool {
        self.budget_exhausted.load(Ordering::Relaxed)
    }

    /// How many transport calls this run has made.
    pub fn fetched(&self) -> usize {
        self.fetched.load(Ordering::Relaxed)
    }

    /// Ask the transport: cache first, then only what is missing, in batches, paced and budgeted.
    ///
    /// Returns whatever it has. A rate limit stops *further fetching*, never the answers already in
    /// hand — a partial answer is worth more than a tidy one.
    pub async fn resolve_requests<T: Transport + ?Sized>(
        &self,
        transport: &T,
        requests: &[Request],
    ) -> Answers {
        let mut answers = Answers::new();
        if requests.is_empty() {
            return answers;
        }
        self.planned.fetch_add(requests.len(), Ordering::Relaxed);

        for ((provider, kind, shape), queries) in plan_groups(requests) {
            let misses = self.cached(provider, &kind, &shape, queries, &mut answers).await;
            if misses.is_empty() || self.offline {
                continue;
            }
            if !self
                .fetch_misses(transport, provider, &kind, &shape, misses, &mut answers)
                .await
            {
                // Rate limited: an instruction, not a fact. Stop asking anything at all.
                break;
            }
        }

        answers
    }

    /// Read one group from the cache and return the queries it did not answer.
    async fn cached(
        &self,
        provider: &str,
        kind: &str,
        shape: &str,
        queries: Vec<String>,
        answers: &mut Answers,
    ) -> Vec<String> {
        let hits = match self.cache.get_many(provider, kind, shape, &queries).await {
            Ok(hits) => hits,
            Err(error) => {
                // A cache is an optimisation, not a source of truth. Returning "no misses" here
                // dropped the group's whole planned set on the floor: the run reported zero claims
                // and nothing said why. Treat an unreadable cache as an empty one and let the
                // provider answer; the write after the fetch will log its own failure if it is
                // still broken.
                tracing::warn!(%error, "could not read the cache; treating the group as uncached");
                return queries;
            }
        };
        self.cache_hits.fetch_add(hits.len(), Ordering::Relaxed);
        for (query, answer) in hits {
            answers.insert(
                (provider.to_string(), kind.to_string(), shape.to_string(), query),
                Raw {
                    outcome: answer.outcome,
                    payload: answer.payload,
                },
            );
        }
        queries
            .into_iter()
            .filter(|query| {
                !answers.contains_key(&(
                    provider.to_string(),
                    kind.to_string(),
                    shape.to_string(),
                    query.clone(),
                ))
            })
            .collect()
    }

    /// Fetch a group's misses in batches. Returns false when the provider told us to stop.
    async fn fetch_misses<T: Transport + ?Sized>(
        &self,
        transport: &T,
        provider: &'static str,
        kind: &str,
        shape: &str,
        misses: Vec<String>,
        answers: &mut Answers,
    ) -> bool {
        for chunk in misses.chunks(transport.batch_size(kind)) {
            if !self.take_call().await {
                self.budget_exhausted.store(true, Ordering::Relaxed);
                break;
            }
            let batch: Vec<Request> = chunk
                .iter()
                .map(|query| Request {
                    provider,
                    kind: kind.to_string(),
                    shape: shape.to_string(),
                    query: query.clone(),
                })
                .collect();
            self.pacer.lock().await.pace().await;
            self.fetched.fetch_add(1, Ordering::Relaxed);

            match transport.fetch(&batch).await {
                Ok(results) => {
                    for (query, raw) in results {
                        if let Err(error) = self
                            .cache
                            .put(
                                provider,
                                kind,
                                shape,
                                &query,
                                &Answer {
                                    outcome: raw.outcome,
                                    payload: raw.payload.clone(),
                                },
                            )
                            .await
                        {
                            tracing::warn!(%error, "could not write the cache");
                        }
                        answers.insert(
                            (
                                provider.to_string(),
                                kind.to_string(),
                                shape.to_string(),
                                query,
                            ),
                            raw,
                        );
                    }
                }
                Err(FetchError::RateLimited { retry_after_secs }) => {
                    // An instruction, not a fact. Do not cache it, do not retry, do not continue:
                    // WDQS temporarily bans clients that keep requesting after a 429.
                    self.rate_limited_after
                        .store(retry_after_secs, Ordering::Relaxed);
                    return false;
                }
                Err(FetchError::Failed(_)) => {
                    // Not a fact about the world either: cache it briefly so a sick provider is not
                    // hammered, then move on.
                    for query in chunk {
                        if let Err(error) = self
                            .cache
                            .put(
                                provider,
                                kind,
                                shape,
                                query,
                                &Answer {
                                    outcome: Outcome::Failed,
                                    payload: None,
                                },
                            )
                            .await
                        {
                            tracing::warn!(%error, "could not write the cache");
                        }
                        answers.insert(
                            (
                                provider.to_string(),
                                kind.to_string(),
                                shape.to_string(),
                                query.clone(),
                            ),
                            Raw {
                                outcome: Outcome::Failed,
                                payload: None,
                            },
                        );
                        self.failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        true
    }

    /// An answer arrived but told us nothing usable. Re-ask it sooner than the default week: a stale
    /// negative is indistinguishable from a fact, and a degraded answer looks exactly like a real one.
    pub async fn unproductive(&self, request: &Request) {
        if let Err(error) = self
            .cache
            .shorten(
                request.provider,
                &request.kind,
                &request.shape,
                &request.query,
                UNPRODUCTIVE_TTL,
            )
            .await
        {
            tracing::warn!(%error, query = %request.query, "could not shorten an unproductive answer");
        }
    }

    pub(crate) fn absorb(&self, report: &mut Report) {
        report.planned = self.planned.load(Ordering::Relaxed);
        report.cache_hits = self.cache_hits.load(Ordering::Relaxed);
        report.fetched = self.fetched.load(Ordering::Relaxed);
        report.failed = self.failed.load(Ordering::Relaxed);
        report.budget_exhausted = self.budget_exhausted();
        report.rate_limited_after = self.rate_limited_after();
    }
}

/// Group planned requests by what a provider can answer in one call, deduped.
///
/// Keyed by `(provider, kind, shape)`, not by query: `title` and `label` ask about the same string and
/// the answers are different things. Keyed by query alone, the label group shadowed the title group
/// and the title requests were never sent.
pub(crate) fn plan_groups(requests: &[Request]) -> BTreeMap<(&'static str, String, String), Vec<String>> {
    let mut groups: BTreeMap<(&'static str, String, String), Vec<String>> = BTreeMap::new();
    for request in requests {
        groups
            .entry((
                request.provider,
                request.kind.clone(),
                request.shape.clone(),
            ))
            .or_default()
            .push(request.query.clone());
    }
    for queries in groups.values_mut() {
        queries.sort();
        queries.dedup();
    }
    groups
}

/// A floor between transport calls.
///
/// WDQS meters processing time rather than request count, but a floor is the simplest thing that
/// cannot be wrong. It lives for a whole run, so two passes back to back do not each start afresh.

struct Pacer {
    after: Option<std::time::Instant>,
    interval: std::time::Duration,
}

impl Pacer {
    fn new(interval: std::time::Duration) -> Self {
        Self {
            after: None,
            interval,
        }
    }

    async fn pace(&mut self) {
        if let Some(previous) = self.after {
            let ready = previous + self.interval;
            let now = std::time::Instant::now();
            if ready > now {
                tokio::time::sleep(ready - now).await;
            }
        }
        self.after = Some(std::time::Instant::now());
    }
}

