//! The provider's only door to the world: cache first, then a paced, budgeted fetch.
//!
//! A provider cannot fetch. It hands [`Gateway::resolve_requests`] the questions it wants answered,
//! and this module decides what is already known, what the budget allows, and when the provider has
//! asked us to stop. That is what lets one own its own strategy — including several passes — without
//! being able to bypass the cache, the pace or the budget.

use std::collections::BTreeMap;
use std::fmt::Display;

use super::allowance::{Allowance, Refusal};
use super::cache::FetchCache;
use super::transport::{Answer, Answers, FetchError, Outcome, Request, Transport};
use super::stats::{Counters, Stats};

/// The one door.
///
/// It knows nothing about requests in general: a caller brings its own [`Transport`], and the request
/// kind and payload types come from it. Everything it may *do* is its three parameters.
pub struct Gateway<'a> {
    cache: Option<&'a FetchCache<'a>>,
    allowance: Option<&'a Allowance>,
    fail_fast: bool,
    counters: Counters,
}

impl<'a> Gateway<'a> {
    /// `cache` — answers already on disk. `None` asks every time and stores nothing.
    ///
    /// `allowance` — permission to call. `None` never fetches, so nothing can be refused: a cache-only
    /// run, which is how the graph is built locally.
    ///
    /// `fail_fast` — what a refusal does. `true` ends the walk (the cache is durable, so the groups it
    /// skips are read by the next run at no network cost); `false` keeps reading the remaining groups'
    /// caches and fetches nothing more.
    ///
    /// The three are independent: with no allowance nothing exhausts, so `fail_fast` is moot; with no
    /// cache nothing can be harvested, so it is moot there too.
    pub fn new(
        cache: Option<&'a FetchCache<'a>>,
        allowance: Option<&'a Allowance>,
        fail_fast: bool,
    ) -> Self {
        Self {
            cache,
            allowance,
            fail_fast,
            counters: Counters::default(),
        }
    }

    /// How many transport calls this run has made.
    pub fn fetched(&self) -> usize {
        self.counters.snapshot().fetched
    }

    /// Everything the run has spent and found. One read, at the end.
    pub fn stats(&self) -> Stats {
        self.counters.snapshot()
    }

    /// Ask the transport: cache first, then only what is missing, in batches, paced and budgeted.
    ///
    /// Returns whatever it has, and stops at the first refusal. The cache is durable, so a group not
    /// read this run is read by the next one at no network cost: carrying on would only add a wide
    /// query per remaining group to a run that has already spent what it may, and blur the report
    /// that says the budget was the limit.
    pub async fn resolve_requests<T: Transport>(
        &self,
        transport: &T,
        requests: &[Request<T::Kind>],
    ) -> Answers<T::Kind, T::Payload> {
        let mut answers = Answers::new();
        if requests.is_empty() {
            return answers;
        }
        self.counters.count_requested(requests.len());

        for ((provider, kind, shape), queries) in plan_groups(requests) {
            let uncached = self
                .uncached_queries(provider, &kind, &shape, queries, &mut answers)
                .await;
            if uncached.is_empty() {
                continue;
            }
            match self
                .fetch_misses(transport, provider, &kind, &shape, uncached, &mut answers)
                .await
            {
                Ok(()) => {}
                // Refused: the run is over. The cache is durable, so the groups skipped here are read
                // by the next run at no network cost, and the report can say the budget was the limit.
                Err(_) if self.fail_fast => break,
                // Refused, but not stopping: read the rest of the cache and fetch nothing more.
                Err(_) => continue,
            }
        }

        answers
    }

    /// Read one group from the cache into `answers`, and return the queries it did not answer.
    async fn uncached_queries<
        K: Clone + Eq + std::hash::Hash + Display,
        P: serde::de::DeserializeOwned,
    >(
        &self,
        provider: &'static str,
        kind: &K,
        shape: &str,
        queries: Vec<String>,
        answers: &mut Answers<K, P>,
    ) -> Vec<String> {
        let Some(cache) = self.cache else {
            // No cache at all: every query is a miss, and nothing is written back.
            return queries;
        };
        let hits = match cache.get_many(provider, kind, shape, &queries).await {
            Ok(hits) => hits,
            Err(error) => {
                // A cache is an optimisation, not a source of truth. Returning "no misses" here
                // dropped the group's whole planned set on the floor: the run reported zero results
                // and nothing said why. Treat an unreadable cache as an empty one and let the
                // provider answer; the write after the fetch will log its own failure if it is
                // still broken.
                tracing::warn!(%error, "could not read the cache; treating the group as uncached");
                return queries;
            }
        };
        self.counters.hit(hits.len());
        let misses: Vec<String> = queries
            .into_iter()
            .filter(|query| !hits.contains_key(query))
            .collect();
        for (query, answer) in hits {
            answers.insert(
                Request {
                    provider,
                    kind: kind.clone(),
                    shape: shape.to_string(),
                    query,
                },
                answer,
            );
        }
        misses
    }

    /// Fetch a group's misses in batches, or say why it stopped.
    async fn fetch_misses<T: Transport>(
        &self,
        transport: &T,
        provider: &'static str,
        kind: &T::Kind,
        shape: &str,
        misses: Vec<String>,
        answers: &mut Answers<T::Kind, T::Payload>,
    ) -> Result<(), Refusal> {
        // Nothing to spend, nothing spent — and no refusal, because there is nothing to refuse.
        let Some(allowance) = self.allowance else {
            return Ok(());
        };
        let cost = transport.cost(kind);
        for chunk in misses.chunks(transport.batch_size(kind)) {
            // Paced once per batch, and charged for every request the batch will make.
            allowance.take(cost).await?;
            let batch: Vec<Request<T::Kind>> = chunk
                .iter()
                .map(|query| Request {
                    provider,
                    kind: kind.clone(),
                    shape: shape.to_string(),
                    query: query.clone(),
                })
                .collect();
            self.counters.fetch(cost);

            match transport.fetch(&batch).await {
                Ok(results) => {
                    for (query, answer) in results {
                        if let Some(cache) = self.cache
                            && let Err(error) = cache.put(provider, kind, shape, &query, &answer).await
                        {
                            tracing::warn!(%error, "could not write the cache");
                        }
                        answers.insert(
                            Request {
                                provider,
                                kind: kind.clone(),
                                shape: shape.to_string(),
                                query,
                            },
                            answer,
                        );
                    }
                }
                Err(FetchError::RateLimited { retry_after_secs }) => {
                    // An instruction, not a fact. Do not cache it, do not retry: latch it on the
                    // allowance, so no later batch, stage or provider asks again. One query endpoint
                    // temporarily bans clients that keep requesting after a 429.
                    allowance.rate_limited(retry_after_secs);
                    return Err(Refusal::RateLimited { retry_after_secs });
                }
                Err(FetchError::Failed(_)) => {
                    // Not a fact about the world either: cache it briefly so a sick provider is not
                    // hammered, then move on.
                    for query in chunk {
                        let answer = Answer::<T::Payload>::nothing(Outcome::Failed);
                        if let Some(cache) = self.cache
                            && let Err(error) = cache.put(provider, kind, shape, query, &answer).await
                        {
                            tracing::warn!(%error, "could not write the cache");
                        }
                        answers.insert(
                            Request {
                                provider,
                                kind: kind.clone(),
                                shape: shape.to_string(),
                                query: query.clone(),
                            },
                            answer,
                        );
                        self.counters.fail();
                    }
                }
            }
        }
        Ok(())
    }

}

/// Group planned requests by what a provider can answer in one call, deduped.
///
/// Keyed by `(provider, kind, shape)`, not by query: two kinds ask about the same string and the
/// answers are different things. Keyed by query alone, one group shadowed another and its requests
/// were never sent.
pub(crate) fn plan_groups<K: Clone + Ord>(
    requests: &[Request<K>],
) -> BTreeMap<(&'static str, K, String), Vec<String>> {
    let mut groups: BTreeMap<(&'static str, K, String), Vec<String>> = BTreeMap::new();
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

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::Mutex;

    use sea_orm::{EntityTrait, PaginatorTrait};

    use crate::test_support::db_or_skip;

    use super::*;
    use crate::fetch::allowance::Refusal;

    /// A kind and a payload invented here, on purpose: if this module needed to know either one, the
    /// test would not compile against its own vocabulary.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    enum Probe {
        Alpha,
        Beta,
    }

    impl fmt::Display for Probe {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                Self::Alpha => "alpha",
                Self::Beta => "beta",
            })
        }
    }

    fn request(kind: Probe, shape: &str, query: &str) -> Request<Probe> {
        Request {
            provider: "probe",
            kind,
            shape: shape.to_string(),
            query: query.to_string(),
        }
    }

    fn found() -> Answer<String> {
        Answer {
            outcome: Outcome::Found,
            body: Some(String::new()),
        }
    }

    /// The transport the gateway sees: records what it was asked, answers from a script. An empty
    /// script answers every batch successfully.
    #[derive(Default)]
    struct Fake {
        seen: Mutex<Vec<String>>,
        script: Mutex<std::collections::VecDeque<Result<(), FetchError>>>,
    }

    impl Fake {
        fn new(script: Vec<Result<(), FetchError>>) -> Self {
            Self {
                seen: Mutex::new(Vec::new()),
                script: Mutex::new(script.into()),
            }
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Transport for Fake {
        type Kind = Probe;
        type Payload = String;

        async fn fetch(
            &self,
            requests: &[Request<Probe>],
        ) -> Result<Vec<(String, Answer<String>)>, FetchError> {
            self.seen
                .lock()
                .unwrap()
                .extend(requests.iter().map(|request| request.query.clone()));
            match self.script.lock().unwrap().pop_front() {
                Some(Err(error)) => Err(error),
                _ => Ok(requests
                    .iter()
                    .map(|request| (request.query.clone(), found()))
                    .collect()),
            }
        }
    }

    /// Two kinds ask about the same string and the answers are different things, so the group key
    /// includes the kind — keyed by query alone one group once swallowed another.
    #[test]
    fn plan_groups_key_by_kind_and_collapse_duplicates() {
        let requests = [
            request(Probe::Alpha, "v1", "query one"),
            request(Probe::Alpha, "v1", "query one"),
            request(Probe::Beta, "v1", "query one"),
            request(Probe::Beta, "v1", "b"),
            request(Probe::Beta, "v1", "a"),
            request(Probe::Beta, "v1", "b"),
        ];
        let groups = plan_groups(&requests);

        assert_eq!(groups.len(), 2, "one group per (provider, kind, shape)");
        assert_eq!(
            groups[&("probe", Probe::Alpha, "v1".into())],
            vec!["query one"],
            "duplicates collapse"
        );
        assert_eq!(
            groups[&("probe", Probe::Beta, "v1".into())],
            vec!["a", "b", "query one"],
            "the same string under another kind is its own group, sorted and deduped"
        );
    }

    /// A hit is answered without asking the provider: that is the whole reason the cache is here.
    #[tokio::test]
    async fn a_cache_hit_never_reaches_the_transport() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        cache
            .put(
                "probe",
                &Probe::Alpha,
                "v1",
                "hit",
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(String::new()),
                },
            )
            .await
            .unwrap();

        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);
        let fake = Fake::default();
        let answers = gateway
            .resolve_requests(
                &fake,
                &[
                    request(Probe::Alpha, "v1", "hit"),
                    request(Probe::Alpha, "v1", "miss"),
                ],
            )
            .await;

        assert_eq!(gateway.stats().cache_hits, 1);
        assert_eq!(gateway.stats().fetched, 1, "only the miss is fetched");
        assert_eq!(fake.seen(), vec!["miss".to_string()]);
        assert_eq!(answers.len(), 2);
    }

    /// A rate limit on the second batch stops the fetching but keeps what the first one answered.
    #[tokio::test]
    async fn a_rate_limit_keeps_what_was_fetched_and_stops() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);
        // `BATCH` is 2 under test, so four queries are two calls.
        let fake = Fake::new(vec![
            Ok(()),
            Err(FetchError::RateLimited {
                retry_after_secs: Some(30),
            }),
        ]);
        let requests: Vec<Request<Probe>> = ["rate-a", "rate-b", "rate-c", "rate-d"]
            .iter()
            .map(|query| request(Probe::Alpha, "v1", query))
            .collect();

        let answers = gateway.resolve_requests(&fake, &requests).await;

        assert_eq!(gateway.stats().fetched, 2);
        assert_eq!(
            allowance.refused(),
            Some(Refusal::RateLimited {
                retry_after_secs: Some(30)
            })
        );
        assert_eq!(answers.len(), 2, "the first batch survives");
    }

    /// A failure is not a fact about the world, so it is cached briefly: the next caller in the same
    /// process must not re-ask a provider that just failed.
    #[tokio::test]
    async fn a_failed_fetch_is_cached_and_counted() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);
        let fake = Fake::new(vec![Err(FetchError::Failed("boom".into()))]);
        let requests = [
            request(Probe::Alpha, "v1", "fail-a"),
            request(Probe::Alpha, "v1", "fail-b"),
        ];

        let answers = gateway.resolve_requests(&fake, &requests).await;
        assert!(answers.values().all(|raw| raw.outcome == Outcome::Failed));
        assert_eq!(gateway.stats().failed, 2);

        let second_allowance = Allowance::new(8, std::time::Duration::ZERO);
        let second = Gateway::new(Some(&cache), Some(&second_allowance), true);
        let again = second.resolve_requests(&fake, &requests).await;
        assert_eq!(second.stats().cache_hits, 2, "the failure was cached");
        assert_eq!(second.stats().fetched, 0);
        assert!(again.values().all(|raw| raw.outcome == Outcome::Failed));
    }

    /// A call that fans out into two HTTP requests is charged both, and the report counts requests
    /// rather than batches. `BATCH` is 2 under test, so four queries are two calls: at one charge per
    /// call a budget of three would fetch both, and at two it fetches one.
    #[tokio::test]
    async fn a_call_that_fans_out_charges_every_request() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let allowance = Allowance::new(3, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);
        let doubled = Doubled(Fake::default());

        let requests: Vec<Request<Probe>> = ["one", "two", "three", "four"]
            .iter()
            .map(|query| request(Probe::Alpha, "v1", query))
            .collect();
        let answers = gateway.resolve_requests(&doubled, &requests).await;

        assert_eq!(answers.len(), 2, "one batch of two, then refused");
        assert_eq!(gateway.stats().fetched, 2, "two HTTP requests, not one call");
        assert_eq!(allowance.refused(), Some(Refusal::Budget));
    }

    /// One call, two HTTP requests.
    struct Doubled(Fake);

    #[async_trait::async_trait]
    impl Transport for Doubled {
        type Kind = Probe;
        type Payload = String;

        fn cost(&self, _kind: &Probe) -> usize {
            2
        }

        async fn fetch(
            &self,
            requests: &[Request<Probe>],
        ) -> Result<Vec<(String, Answer<String>)>, FetchError> {
            self.0.fetch(requests).await
        }
    }

    /// The budget is spent *before* the call, so a zero budget issues none — and says why.
    #[tokio::test]
    async fn a_spent_budget_issues_no_call() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let allowance = Allowance::new(0, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);
        let fake = Fake::default();

        let answers = gateway
            .resolve_requests(&fake, &[request(Probe::Alpha, "v1", "budget")])
            .await;

        assert!(answers.is_empty());
        assert_eq!(allowance.refused(), Some(Refusal::Budget), "the refusal is recorded");
        assert_eq!(gateway.stats().fetched, 0);
        assert!(fake.seen().is_empty());
    }

    /// A refusal ends the walk.
    ///
    /// The cache is durable, so the groups a stopped run never reached are read by the next run at no
    /// network cost. Carrying on would add one wide query per remaining group to a run that has
    /// already spent what it may — and hide, behind a longer report, that the budget is what stopped
    /// it.
    #[tokio::test]
    async fn a_spent_budget_ends_the_walk() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        cache
            .put(
                "probe",
                &Probe::Beta,
                "v1",
                "warm",
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(String::new()),
                },
            )
            .await
            .unwrap();
        let allowance = Allowance::new(0, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);
        let fake = Fake::default();

        // Alpha sorts first and misses, so the budget is gone before Beta is reached — and Beta's
        // cached answer is *not* collected, because the run stopped rather than keeping reading.
        let answers = gateway
            .resolve_requests(
                &fake,
                &[
                    request(Probe::Alpha, "v1", "cold"),
                    request(Probe::Beta, "v1", "warm"),
                ],
            )
            .await;

        assert!(
            answers.is_empty(),
            "the walk stopped before the cached group: {answers:?}"
        );
        assert_eq!(allowance.refused(), Some(Refusal::Budget));
        assert!(fake.seen().is_empty(), "nothing was fetched");
    }

    /// `fail_fast: false` — a refusal stops spending but not reading, so the walk continues and
    /// harvests the remaining groups from the cache. The other side of `a_spent_budget_ends_the_walk`.
    #[tokio::test]
    async fn without_fail_fast_the_rest_of_the_walk_is_read_from_the_cache() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        cache
            .put(
                "probe",
                &Probe::Beta,
                "v1",
                "warm",
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(String::new()),
                },
            )
            .await
            .unwrap();
        let allowance = Allowance::new(0, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), false);
        let fake = Fake::default();

        let answers = gateway
            .resolve_requests(
                &fake,
                &[
                    request(Probe::Alpha, "v1", "cold"),
                    request(Probe::Beta, "v1", "warm"),
                ],
            )
            .await;

        assert_eq!(
            answers.len(),
            1,
            "the cached group is still read: {answers:?}"
        );
        assert_eq!(allowance.refused(), Some(Refusal::Budget));
        assert!(fake.seen().is_empty(), "nothing was fetched");
    }

    /// `cache: None` — every request is a miss, and nothing is written back.
    #[tokio::test]
    async fn without_a_cache_every_request_is_fetched_and_nothing_is_stored() {
        db_or_skip!(db);
        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(None, Some(&allowance), true);
        let fake = Fake::default();

        let answers = gateway
            .resolve_requests(&fake, &[request(Probe::Alpha, "v1", "fresh")])
            .await;

        assert_eq!(answers.len(), 1);
        assert_eq!(fake.seen(), vec!["fresh".to_string()], "asked, not read");
        assert_eq!(
            crate::model::fetch_cache::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            0,
            "a cacheless run stores nothing"
        );
    }

    /// Cache only: no allowance, so every group is read and nothing is fetched — the same walk a
    /// refusal *stops*, which is why "may never fetch" is not a refusal.
    #[tokio::test]
    async fn no_allowance_reads_every_group_and_fetches_nothing() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        for (kind, query) in [(Probe::Alpha, "first-hit"), (Probe::Beta, "second-hit")] {
            cache
                .put(
                    "probe",
                    &kind,
                    "v1",
                    query,
                    &Answer {
                        outcome: Outcome::Found,
                        body: Some(String::new()),
                    },
                )
                .await
                .unwrap();
        }
        let gateway = Gateway::new(Some(&cache), None, false);
        let fake = Fake::default();

        // Two groups, with a miss in the first: both cached answers still come back.
        let answers = gateway
            .resolve_requests(
                &fake,
                &[
                    request(Probe::Alpha, "v1", "first-hit"),
                    request(Probe::Alpha, "v1", "first-miss"),
                    request(Probe::Beta, "v1", "second-hit"),
                ],
            )
            .await;

        assert_eq!(answers.len(), 2, "{answers:?}");
        assert_eq!(gateway.stats().cache_hits, 2);
        assert!(fake.seen().is_empty(), "no allowance never calls");
    }
}
