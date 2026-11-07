//! The resolver's only door to the world: cache first, then a paced, budgeted fetch.
//!
//! A resolver cannot fetch. It hands `resolve_requests` the questions it wants answered, and this
//! module decides what is already known, what the budget allows, and when the provider has asked us
//! to stop. That is what lets one own its own strategy — including several passes — without being
//! able to bypass the cache, the pace or the budget.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::cache::{Answer, Cache, Outcome, UNPRODUCTIVE_TTL};
use super::contract::{Answers, FetchError, Kind, Raw, Request, Transport};
use super::engine::Options;

/// No rate limit seen. Distinguishes "the provider did not ask us to wait" from "wait zero seconds".
const NOT_LIMITED: u64 = u64::MAX;

/// What one run has spent and found, once it is over.
///
/// The counters are written by any resolver at any time and read exactly once, at the end of the
/// run. A named type rather than four loose atomics on the gateway: the report is assembled from one
/// snapshot, and the gateway no longer needs to know what a report is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Requests handed to `resolve_requests`, including cache hits.
    pub planned: usize,
    /// Answers the cache already had.
    pub cache_hits: usize,
    /// Transport *calls*, not queries: one call can carry a batch.
    pub fetched: usize,
    /// Queries whose fetch failed.
    pub failed: usize,
}

#[derive(Default)]
struct Counters {
    planned: AtomicUsize,
    cache_hits: AtomicUsize,
    fetched: AtomicUsize,
    failed: AtomicUsize,
}

impl Counters {
    fn plan(&self, queries: usize) {
        self.planned.fetch_add(queries, Ordering::Relaxed);
    }

    fn hit(&self, answers: usize) {
        self.cache_hits.fetch_add(answers, Ordering::Relaxed);
    }

    fn fetch(&self) {
        self.fetched.fetch_add(1, Ordering::Relaxed);
    }

    fn fail(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> Stats {
        Stats {
            planned: self.planned.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            fetched: self.fetched.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        }
    }
}

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
    counters: Counters,
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
            counters: Counters::default(),
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
        self.counters.snapshot().fetched
    }

    /// Everything the run has spent and found. One read, at the end.
    pub(crate) fn stats(&self) -> Stats {
        self.counters.snapshot()
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
        self.counters.plan(requests.len());

        for ((provider, kind, shape), queries) in plan_groups(requests) {
            let misses = self
                .cached(provider, &kind, &shape, queries, &mut answers)
                .await;
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
        kind: &Kind,
        shape: &str,
        queries: Vec<String>,
        answers: &mut Answers,
    ) -> Vec<String> {
        let hits = match self
            .cache
            .get_many(provider, &kind.to_key(), shape, &queries)
            .await
        {
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
        self.counters.hit(hits.len());
        for (query, answer) in hits {
            answers.insert(
                (provider.to_string(), kind.clone(), shape.to_string(), query),
                Raw {
                    outcome: answer.outcome,
                    candidates: super::cache::candidates(answer.payload.as_ref()),
                },
            );
        }
        queries
            .into_iter()
            .filter(|query| {
                !answers.contains_key(&(
                    provider.to_string(),
                    kind.clone(),
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
        kind: &Kind,
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
                    kind: kind.clone(),
                    shape: shape.to_string(),
                    query: query.clone(),
                })
                .collect();
            self.pacer.lock().await.pace().await;
            self.counters.fetch();

            match transport.fetch(&batch).await {
                Ok(results) => {
                    for (query, raw) in results {
                        if let Err(error) = self
                            .cache
                            .put(
                                provider,
                                &kind.to_key(),
                                shape,
                                &query,
                                &Answer {
                                    outcome: raw.outcome,
                                    payload: Some(super::cache::payload_of(&raw.candidates)),
                                },
                            )
                            .await
                        {
                            tracing::warn!(%error, "could not write the cache");
                        }
                        answers.insert(
                            (provider.to_string(), kind.clone(), shape.to_string(), query),
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
                                &kind.to_key(),
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
                                kind.clone(),
                                shape.to_string(),
                                query.clone(),
                            ),
                            Raw {
                                outcome: Outcome::Failed,
                                candidates: Vec::new(),
                            },
                        );
                        self.counters.fail();
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
                &request.kind.to_key(),
                &request.shape,
                &request.query,
                UNPRODUCTIVE_TTL,
            )
            .await
        {
            tracing::warn!(%error, query = %request.query, "could not shorten an unproductive answer");
        }
    }
}

/// Group planned requests by what a provider can answer in one call, deduped.
///
/// Keyed by `(provider, kind, shape)`, not by query: `title` and `label` ask about the same string and
/// the answers are different things. Keyed by query alone, the label group shadowed the title group
/// and the title requests were never sent.
pub(crate) fn plan_groups(
    requests: &[Request],
) -> BTreeMap<(&'static str, Kind, String), Vec<String>> {
    let mut groups: BTreeMap<(&'static str, Kind, String), Vec<String>> = BTreeMap::new();
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::test_support::db_or_skip;

    use super::*;

    fn request(kind: Kind, shape: &str, query: &str) -> Request {
        Request {
            provider: "test",
            kind,
            shape: shape.to_string(),
            query: query.to_string(),
        }
    }

    fn options() -> Options {
        Options {
            max_requests: 8,
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        }
    }

    fn found() -> Raw {
        Raw {
            outcome: Outcome::Found,
            candidates: Vec::new(),
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
        async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
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

    /// `title` and `label` ask about the same string and the answers are different things, so the
    /// group key includes the kind — keyed by query alone the label group once swallowed the titles.
    #[test]
    fn plan_groups_key_by_kind_and_collapse_duplicates() {
        let requests = [
            request(Kind::Label, "v1", "Train Dreams"),
            request(Kind::Label, "v1", "Train Dreams"),
            request(Kind::Title, "v1", "Train Dreams"),
            request(Kind::Search, "v1", "b"),
            request(Kind::Search, "v1", "a"),
            request(Kind::Search, "v1", "b"),
        ];
        let groups = plan_groups(&requests);

        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[&("test", Kind::Label, "v1".into())],
            vec!["Train Dreams"]
        );
        assert_eq!(
            groups[&("test", Kind::Title, "v1".into())],
            vec!["Train Dreams"]
        );
        assert_eq!(groups[&("test", Kind::Search, "v1".into())], vec!["a", "b"]);
    }

    /// A hit is answered without asking the provider: that is the whole reason the cache is here.
    #[tokio::test]
    async fn a_cache_hit_never_reaches_the_transport() {
        db_or_skip!(db);
        let cache = Cache::new(&db.conn);
        cache
            .put(
                "test",
                "label",
                "v1",
                "hit",
                &Answer {
                    outcome: Outcome::Found,
                    payload: None,
                },
            )
            .await
            .unwrap();

        let gateway = Gateway::new(&cache, &options());
        let fake = Fake::default();
        let answers = gateway
            .resolve_requests(
                &fake,
                &[
                    request(Kind::Label, "v1", "hit"),
                    request(Kind::Label, "v1", "miss"),
                ],
            )
            .await;

        assert_eq!(gateway.stats().cache_hits, 1);
        assert_eq!(gateway.stats().fetched, 1, "only the miss is fetched");
        assert_eq!(fake.seen(), vec!["miss".to_string()]);
        assert_eq!(answers.len(), 2);
    }

    /// The regression the engine test guards from outside: a rate limit on the second batch stops
    /// the fetching but keeps what the first one answered.
    #[tokio::test]
    async fn a_rate_limit_keeps_what_was_fetched_and_stops() {
        db_or_skip!(db);
        let cache = Cache::new(&db.conn);
        let gateway = Gateway::new(&cache, &options());
        // `BATCH` is 2 under test, so four queries are two calls.
        let fake = Fake::new(vec![
            Ok(()),
            Err(FetchError::RateLimited {
                retry_after_secs: 30,
            }),
        ]);
        let requests: Vec<Request> = ["rate-a", "rate-b", "rate-c", "rate-d"]
            .iter()
            .map(|query| request(Kind::Label, "v1", query))
            .collect();

        let answers = gateway.resolve_requests(&fake, &requests).await;

        assert_eq!(gateway.stats().fetched, 2);
        assert_eq!(gateway.rate_limited_after(), Some(30));
        assert_eq!(answers.len(), 2, "the first batch survives");
    }

    /// A failure is not a fact about the world, so it is cached briefly: the next caller in the same
    /// process must not re-ask a provider that just failed.
    #[tokio::test]
    async fn a_failed_fetch_is_cached_and_counted() {
        db_or_skip!(db);
        let cache = Cache::new(&db.conn);
        let gateway = Gateway::new(&cache, &options());
        let fake = Fake::new(vec![Err(FetchError::Failed("boom".into()))]);
        let requests = [
            request(Kind::Label, "v1", "fail-a"),
            request(Kind::Label, "v1", "fail-b"),
        ];

        let answers = gateway.resolve_requests(&fake, &requests).await;
        assert!(answers.values().all(|raw| raw.outcome == Outcome::Failed));
        assert_eq!(gateway.stats().failed, 2);

        let second = Gateway::new(&cache, &options());
        let again = second.resolve_requests(&fake, &requests).await;
        assert_eq!(second.stats().cache_hits, 2, "the failure was cached");
        assert_eq!(second.stats().fetched, 0);
        assert!(again.values().all(|raw| raw.outcome == Outcome::Failed));
    }

    /// The budget is spent *before* the call, so a zero budget issues none — and says why.
    #[tokio::test]
    async fn a_spent_budget_issues_no_call() {
        db_or_skip!(db);
        let cache = Cache::new(&db.conn);
        let gateway = Gateway::new(
            &cache,
            &Options {
                max_requests: 0,
                min_interval: std::time::Duration::from_millis(0),
                ..Default::default()
            },
        );
        let fake = Fake::default();

        let answers = gateway
            .resolve_requests(&fake, &[request(Kind::Label, "v1", "budget")])
            .await;

        assert!(answers.is_empty());
        assert!(gateway.budget_exhausted(), "the refusal is recorded");
        assert_eq!(gateway.stats().fetched, 0);
        assert!(fake.seen().is_empty());
    }

    /// Offline answers from the cache and nothing else — the pass that makes rebuilding free.
    #[tokio::test]
    async fn offline_answers_from_the_cache_alone() {
        db_or_skip!(db);
        let cache = Cache::new(&db.conn);
        cache
            .put(
                "test",
                "label",
                "v1",
                "offline-hit",
                &Answer {
                    outcome: Outcome::Found,
                    payload: None,
                },
            )
            .await
            .unwrap();
        let gateway = Gateway::new(
            &cache,
            &Options {
                offline: true,
                min_interval: std::time::Duration::from_millis(0),
                ..Default::default()
            },
        );
        let fake = Fake::default();

        let answers = gateway
            .resolve_requests(
                &fake,
                &[
                    request(Kind::Label, "v1", "offline-hit"),
                    request(Kind::Label, "v1", "offline-miss"),
                ],
            )
            .await;

        assert_eq!(answers.len(), 1);
        assert_eq!(gateway.stats().cache_hits, 1);
        assert!(fake.seen().is_empty(), "offline never calls");
    }
}
