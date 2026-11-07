//! What a resolver is, and the engine that runs it.
//!
//! A `Resolver` is one authority plus the strategy for getting claims out of it: it is handed a
//! batch of entities and returns claims about them. How it gets there — one request or a ladder of
//! them, one pass or several — is its own business.
//!
//! Adding an authority means writing one `Resolver`. It cannot forget the cache, because
//! `ResolutionContext::resolve_requests` is the only way to reach the world: there is no other door.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::Value;

use super::cache::{Answer, Cache, Outcome, UNPRODUCTIVE_TTL};
use crate::{database, model};

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

/// Answers keyed by `(kind, shape, query)`. `title` and `label` ask about the same string and the
/// answers are different things, so the kind has to be in the key.
pub type Answers = HashMap<(String, String, String), Raw>;

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
        ctx: &ResolutionContext<'_>,
    ) -> Vec<(i32, Claim)>;
}

/// Transport calls a plain resolve is allowed to make. Exists so that omitting the budget does real
/// work instead of quietly resolving nothing.
pub const DEFAULT_MAX_REQUESTS: usize = 200;

#[derive(Debug, Clone)]
pub struct Options {
    /// How many candidate entities to consider.
    pub limit: Option<usize>,
    /// Transport calls allowed in this run — the anti-spam budget.
    pub max_requests: usize,
    /// Minimum wall-clock gap between transport calls. One request per second keeps a walk of a few
    /// thousand labels to a few minutes while staying far below WDQS's processing-time budget.
    pub min_interval: std::time::Duration,
    pub offline: bool,
    /// Resolve only this entity — for debugging one case without walking the corpus.
    pub only: Option<i32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            limit: None,
            max_requests: DEFAULT_MAX_REQUESTS,
            min_interval: std::time::Duration::from_secs(1),
            offline: false,
            only: None,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub candidates: usize,
    pub planned: usize,
    pub cache_hits: usize,
    /// Transport calls made.
    pub fetched: usize,
    pub failed: usize,
    pub claims: usize,
    /// Entities that gained at least one claim.
    pub entities_resolved: usize,
    pub budget_exhausted: bool,
    /// Set when the provider asked us to back off: the run stopped instead of risking a ban.
    pub rate_limited_after: Option<u64>,
    pub offline: bool,
}

/// No rate limit seen. Distinguishes "the provider did not ask us to wait" from "wait zero seconds".
const NOT_LIMITED: u64 = u64::MAX;

/// What a resolver is given.
///
/// It is the only door to the world: a resolver cannot fetch, only resolve requests. That is what
/// lets one own its own strategy — including several passes — without being able to bypass the
/// cache, the pace or the budget.
pub struct ResolutionContext<'a> {
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

impl<'a> ResolutionContext<'a> {
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
                tracing::warn!(%error, "could not read the cache");
                return Vec::new();
            }
        };
        self.cache_hits.fetch_add(hits.len(), Ordering::Relaxed);
        for (query, answer) in hits {
            answers.insert(
                (kind.to_string(), shape.to_string(), query),
                Raw {
                    outcome: answer.outcome,
                    payload: answer.payload,
                },
            );
        }
        queries
            .into_iter()
            .filter(|query| {
                !answers.contains_key(&(kind.to_string(), shape.to_string(), query.clone()))
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
                        answers.insert((kind.to_string(), shape.to_string(), query), raw);
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
                            (kind.to_string(), shape.to_string(), query.clone()),
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

    fn absorb(&self, report: &mut Report) {
        report.planned = self.planned.load(Ordering::Relaxed);
        report.cache_hits = self.cache_hits.load(Ordering::Relaxed);
        report.fetched = self.fetched.load(Ordering::Relaxed);
        report.failed = self.failed.load(Ordering::Relaxed);
        report.budget_exhausted = self.budget_exhausted();
        report.rate_limited_after = self.rate_limited_after();
    }
}

/// The harness: it holds the resolvers, gives them a context, and stores what they return.
///
/// It knows nothing about stages, tiers or ladders. A resolver that wants two passes takes them; the
/// engine only orders resolvers and merges their claims.
pub struct ResolutionEngine<'a> {
    db: &'a database::DbHandle,
    options: Options,
    resolvers: Vec<Box<dyn Resolver>>,
}

impl<'a> ResolutionEngine<'a> {
    pub fn new(db: &'a database::DbHandle, options: Options) -> Self {
        Self {
            db,
            options,
            resolvers: Vec::new(),
        }
    }

    /// Register an authority. This is the whole cost of adding one.
    pub fn with(mut self, resolver: Box<dyn Resolver>) -> Self {
        self.resolvers.push(resolver);
        self
    }

    pub async fn run(self) -> anyhow::Result<Report> {
        let entities = candidates(&self.db.conn, self.options.limit, self.options.only).await?;
        let cache = Cache::new(&self.db.conn);
        let ctx = ResolutionContext::new(&cache, &self.options);
        let mut report = Report {
            candidates: entities.len(),
            offline: self.options.offline,
            ..Default::default()
        };
        // An online pass is the maintenance moment: drop answers that aged out long ago. Offline runs
        // leave the table alone, since they neither refresh nor ask anything.
        if !self.options.offline {
            let pruned = cache.prune().await?;
            if pruned > 0 {
                tracing::info!(pruned, "Pruned expired resolution cache rows");
            }
        }

        let mut resolved: HashSet<i32> = HashSet::new();
        let knowledge = crate::knowledge::Knowledge::new(self.db);

        for resolver in &self.resolvers {
            if ctx.rate_limited_after().is_some() {
                break;
            }
            // One entity, one identity: the first claim the graph accepts wins, and the rest of that
            // entity's claims are dropped. The resolver's ordering is its own priority.
            for (knowledge_entity_id, claim) in resolver.resolve(&entities, &ctx).await {
                if resolved.contains(&knowledge_entity_id) {
                    continue;
                }
                if store_claim(
                    self.db,
                    resolver.id(),
                    knowledge_entity_id,
                    claim,
                    &mut report,
                )
                .await?
                {
                    resolved.insert(knowledge_entity_id);
                }
            }
        }

        // A later `proximity merge` picks a survivor by the projected `confidence`, so an entity that
        // just earned its first claim is projected here — otherwise it still reads as the weaker side
        // and loses a fold it should win.
        for knowledge_entity_id in &resolved {
            knowledge.update_projection(*knowledge_entity_id).await?;
        }

        // Resolving stops at evidence. Which holders of an identity are one thing belongs to
        // `proximity scan` and `proximity merge`, which are separate commands run after this one.
        report.entities_resolved = resolved.len();
        ctx.absorb(&mut report);
        Ok(report)
    }
}

/// Group planned requests by what a provider can answer in one call, deduped.
///
/// Keyed by `(provider, kind, shape)`, not by query: `title` and `label` ask about the same string and
/// the answers are different things. Keyed by query alone, the label group shadowed the title group
/// and the title requests were never sent.
fn plan_groups(requests: &[Request]) -> BTreeMap<(&'static str, String, String), Vec<String>> {
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

/// Store one claim as an edge to its identity. Returns whether a new claim was written; an identity
/// another entity already holds is not a conflict, because claims are many-to-one — whether the
/// holders are one thing is [`crate::knowledge::proximity`]'s question.
async fn store_claim(
    db: &database::DbHandle,
    source: &str,
    knowledge_entity_id: i32,
    claim: Claim,
    report: &mut Report,
) -> anyhow::Result<bool> {
    let evidence = crate::knowledge::evidence::Evidence::new(db.conn.clone());
    let identity_id = evidence
        .upsert_identity(&crate::knowledge::evidence::DraftIdentity {
            scheme: claim.scheme,
            external_id: claim.id,
            source: source.to_string(),
            method: Some(claim.method),
            confidence: Some(claim.confidence),
            label: claim.label,
            description: claim.description,
            classes: claim.classes,
        })
        .await?;

    let insert = model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
        knowledge_entity_id: Set(knowledge_entity_id),
        knowledge_identity_id: Set(identity_id),
        basis: Set(claim.basis.as_str().to_string()),
        ..Default::default()
    })
    .exec(&db.conn)
    .await;

    match insert {
        Ok(_) => {
            report.claims += 1;
            Ok(true)
        }
        // This entity already asserts the identity.
        Err(error) => match crate::knowledge::evidence::unique_violation(&error) {
            Some("idx-knowledge_claims_entity_identity") => Ok(false),
            _ => Err(error.into()),
        },
    }
}

/// Entities with no live ontology claim. Note the filter is *ontology*, not *no evidence*: an entity
/// identified only by a platform handle still deserves a QID, because that is how corroboration
/// happens.
async fn candidates(
    conn: &sea_orm::DatabaseConnection,
    limit: Option<usize>,
    only: Option<i32>,
) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
    use model::knowledge_entity as entity;
    use sea_orm::sea_query::{Expr, ExprTrait, Query};

    // Asked already: holds a claim an authority made, rather than only the capture's handle.
    let claimed = Query::select()
        .distinct()
        .column((
            model::knowledge_claims::Entity,
            model::knowledge_claims::Column::KnowledgeEntityId,
        ))
        .from(model::knowledge_claims::Entity)
        .inner_join(
            model::knowledge_identity::Entity,
            Expr::col((
                model::knowledge_claims::Entity,
                model::knowledge_claims::Column::KnowledgeIdentityId,
            ))
            .equals((
                model::knowledge_identity::Entity,
                model::knowledge_identity::Column::Id,
            )),
        )
        .and_where(
            Expr::col((
                model::knowledge_claims::Entity,
                model::knowledge_claims::Column::RetractedAt,
            ))
            .is_null(),
        )
        .and_where(
            Expr::col((
                model::knowledge_identity::Entity,
                model::knowledge_identity::Column::Source,
            ))
            .ne("platform"),
        )
        .to_owned();

    let mut query = entity::Entity::find()
        .filter(entity::Column::MergedIntoId.is_null())
        .filter(Expr::col(entity::Column::Id).not_in_subquery(claimed))
        .order_by_asc(entity::Column::Id);
    if let Some(knowledge_entity_id) = only {
        query = query.filter(entity::Column::Id.eq(knowledge_entity_id));
    }
    if let Some(limit) = limit {
        query = query.limit(limit as u64);
    }
    Ok(query.all(conn).await?)
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::test_support::db_or_skip;

    /// An authority that never touches the network: it answers from a fixed map and counts calls.
    struct Fake {
        answers: HashMap<String, Raw>,
        calls: Arc<AtomicUsize>,
    }

    fn fake_request(entity: &model::knowledge_entity::Model) -> Request {
        Request {
            provider: "fake",
            kind: "label".to_string(),
            shape: "v1".to_string(),
            query: entity.name.clone(),
        }
    }

    #[async_trait::async_trait]
    impl Resolver for Fake {
        // The scheme and source the projection counts as text evidence, so a fake resolve produces
        // the same claims a real one would.
        fn id(&self) -> &'static str {
            "wikidata"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            ctx: &ResolutionContext<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request> = entities.iter().map(fake_request).collect();
            let answers = ctx.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&(
                        "label".to_string(),
                        "v1".to_string(),
                        entity.name.clone(),
                    ))?;
                    let id = raw.payload.as_ref()?.get("claim")?.as_str()?;
                    Some((
                        entity.id,
                        Claim {
                            scheme: "wikidata".to_string(),
                            id: id.to_string(),
                            label: Some(entity.name.clone()),
                            description: None,
                            classes: Vec::new(),
                            method: "exact_label".to_string(),
                            confidence: 0.9,
                            basis: crate::knowledge::evidence::ClaimBasis::Text,
                        },
                    ))
                })
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl Transport for Fake {
        async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(requests
                .iter()
                .map(|request| {
                    let raw = self.answers.get(&request.query).cloned().unwrap_or(Raw {
                        outcome: Outcome::Empty,
                        payload: None,
                    });
                    (request.query.clone(), raw)
                })
                .collect())
        }
    }

    /// One observation, linked to an entity by the real linker — the same path production takes.
    async fn seed(db: &database::DbHandle, name: &str) -> i32 {
        let capture = crate::test_support::fixtures::Capture::new(db, "resolve").await;
        let observation = crate::test_support::fixtures::observe(
            db, &capture, 0, name, None, "book", None, None,
        )
        .await;
        crate::test_support::link::link_all(db).await.unwrap();

        model::user_observed_entity::Entity::find_by_id(observation)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .knowledge_entity_id
            .expect("build_core should have linked the observation")
    }

    #[tokio::test]
    async fn a_claim_identifies_the_core_and_a_second_run_asks_nothing() {
        db_or_skip!(db);
        let knowledge_entity_id = seed(&db, "Train Dreams").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let answers = HashMap::from([(
            "Train Dreams".to_string(),
            Raw {
                outcome: Outcome::Found,
                payload: Some(json!({"claim": "Q185971"})),
            },
        )]);

        let first = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers: answers.clone(),
                calls: calls.clone(),
            }))
            .run()
            .await
            .expect("resolve");
        assert_eq!(first.claims, 1, "{first:?}");
        assert_eq!(first.fetched, 1, "{first:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let entity = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entity.status, "identified");
        assert_eq!(entity.confidence, Some(0.9));

        // Second run: the entity now has an ontology claim, so it is not even a candidate.
        let second = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers,
                calls: calls.clone(),
            }))
            .run()
            .await
            .expect("resolve");
        assert_eq!(second.candidates, 0, "{second:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second fetch");
    }

    #[tokio::test]
    async fn an_empty_answer_is_cached_and_not_re_asked() {
        db_or_skip!(db);
        let knowledge_entity_id = seed(&db, "MUME").await;

        let calls = Arc::new(AtomicUsize::new(0));

        let first = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers: HashMap::new(),
                calls: calls.clone(),
            }))
            .run()
            .await
            .expect("resolve");
        assert_eq!(first.fetched, 1, "{first:?}");
        assert_eq!(first.claims, 0);

        // The entity is still a candidate (no claim), but the negative answer is cached.
        let second = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers: HashMap::new(),
                calls: calls.clone(),
            }))
            .run()
            .await
            .expect("resolve");
        assert_eq!(second.candidates, 1, "{second:?}");
        assert_eq!(second.fetched, 0, "{second:?}");
        assert_eq!(second.cache_hits, 1, "{second:?}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the negative was not re-fetched"
        );

        let entity = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entity.status, "provisional");
    }

    /// Two *independent inputs* — a text match and an account link — raise confidence.
    #[tokio::test]
    async fn corroboration_counts_independent_inputs() {
        db_or_skip!(db);
        let knowledge_entity_id = seed(&db, "Nintendo").await;

        crate::test_support::fixtures::claim(
            &db,
            knowledge_entity_id,
            "wikidata",
            "Q8093",
            "wikidata",
            Some("exact_label"),
            Some(0.8),
            "text",
        )
        .await;
        crate::test_support::fixtures::claim(
            &db,
            knowledge_entity_id,
            "x_twitter",
            "nintendo",
            "platform",
            None,
            None,
            "handle",
        )
        .await;

        crate::knowledge::Knowledge::new(&db)
            .update_projection(knowledge_entity_id)
            .await
            .unwrap();

        let entity = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (entity.confidence.unwrap() - 0.9).abs() < 1e-6,
            "{entity:?}"
        );
    }

    /// A handle from a capture and an authority asked about that handle are one chain, however many
    /// providers and schemes they name — the claim's basis says so, so nothing has to infer it.
    #[tokio::test]
    async fn a_handle_basis_claim_is_not_corroboration() {
        db_or_skip!(db);
        let knowledge_entity_id = seed(&db, "Tsoding").await;

        for (source, method, scheme, external_id, confidence) in [
            ("platform", None, "x_twitter", "tsoding", None),
            (
                "wikidata",
                Some("p2002"),
                "wikidata",
                "Q123737256",
                Some(0.95),
            ),
        ] {
            crate::test_support::fixtures::claim(
                &db,
                knowledge_entity_id,
                scheme,
                external_id,
                source,
                method,
                confidence,
                crate::knowledge::evidence::ClaimBasis::Handle.as_str(),
            )
            .await;
        }

        crate::knowledge::Knowledge::new(&db)
            .update_projection(knowledge_entity_id)
            .await
            .unwrap();

        let entity = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (entity.confidence.unwrap() - 0.95).abs() < 1e-6,
            "one chain must not be paid twice: {entity:?}"
        );
    }

    /// A 429 is an instruction, not a fact: stop, cache nothing, and report the Retry-After.
    struct Limited;

    #[async_trait::async_trait]
    impl Resolver for Limited {
        fn id(&self) -> &'static str {
            "limited"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            ctx: &ResolutionContext<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request> = entities.iter().map(fake_request).collect();
            ctx.resolve_requests(self, &requests).await;
            Vec::new()
        }
    }

    #[async_trait::async_trait]
    impl Transport for Limited {
        async fn fetch(&self, _requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            Err(FetchError::RateLimited {
                retry_after_secs: 30,
            })
        }
    }

    /// The regression that mattered: a rate limit on the second batch must not discard the claims
    /// from the first. Fetching stops; the claims already in hand do not.
    struct Flaky {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Resolver for Flaky {
        fn id(&self) -> &'static str {
            "flaky"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            ctx: &ResolutionContext<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request> = entities
                .iter()
                .map(|entity| Request {
                    provider: "flaky",
                    kind: "label".to_string(),
                    shape: "v1".to_string(),
                    query: entity.name.clone(),
                })
                .collect();
            let answers = ctx.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&(
                        "label".to_string(),
                        "v1".to_string(),
                        entity.name.clone(),
                    ))?;
                    (raw.outcome == Outcome::Found).then(|| {
                        (
                            entity.id,
                            Claim {
                                scheme: "wikidata".to_string(),
                                id: format!("Q{}", entity.name.len()),
                                label: Some(entity.name.clone()),
                                description: None,
                                classes: Vec::new(),
                                method: "exact_label".to_string(),
                                confidence: 0.9,
                                basis: crate::knowledge::evidence::ClaimBasis::Text,
                            },
                        )
                    })
                })
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl Transport for Flaky {
        async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) >= 1 {
                return Err(FetchError::RateLimited {
                    retry_after_secs: 60,
                });
            }
            Ok(requests
                .iter()
                .map(|request| {
                    (
                        request.query.clone(),
                        Raw {
                            outcome: Outcome::Found,
                            payload: Some(json!({"candidates": []})),
                        },
                    )
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn a_rate_limit_keeps_the_claims_already_fetched() {
        db_or_skip!(db);
        for name in ["Alpha One", "Beta Two", "Gamma Three", "Delta Four"] {
            seed(&db, name).await;
        }

        let report = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Flaky {
                calls: Arc::new(AtomicUsize::new(0)),
            }))
            .run()
            .await
            .expect("resolve");

        assert_eq!(report.rate_limited_after, Some(60), "{report:?}");
        assert_eq!(
            report.fetched, 2,
            "one successful batch, then the 429: {report:?}"
        );
        assert_eq!(
            report.claims, BATCH,
            "claims from the first batch survive: {report:?}"
        );
    }

    #[tokio::test]
    async fn a_rate_limit_stops_the_run_and_is_not_cached() {
        db_or_skip!(db);
        seed(&db, "Train Dreams").await;

        let report = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Limited))
            .run()
            .await
            .expect("resolve");

        assert_eq!(report.rate_limited_after, Some(30), "{report:?}");
        assert_eq!(report.fetched, 1, "{report:?}");
        assert_eq!(report.claims, 0, "{report:?}");
        assert_eq!(
            model::resolution_cache::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            0,
            "a rate limit must not be recorded as an answer"
        );
    }

    /// Substantiation is a *projected* column, and the merge phase picks a survivor by it. So an
    /// entity that earned its authority claim in this run must be projected before `merge` folds
    /// it — otherwise it still reads as the weaker side and is the one tombstoned.
    #[tokio::test]
    async fn the_side_that_earned_a_claim_this_run_survives_the_fold() {
        db_or_skip!(db);

        // One name, two entities, different illuminations, so a fold is allowed at all.
        let resolved_before =
            crate::test_support::fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let earns_now =
            crate::test_support::fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;

        // One already holds the authority claim; the other has only a handle, which is what makes it
        // a candidate — and a second independent input once the claim arrives.
        crate::test_support::fixtures::claim(
            &db,
            resolved_before,
            "wikidata",
            "Q42",
            "wikidata",
            Some("exact_label"),
            Some(0.9),
            "text",
        )
        .await;
        crate::test_support::fixtures::claim(
            &db,
            earns_now,
            "x_twitter",
            "traindreams",
            "platform",
            None,
            None,
            "handle",
        )
        .await;

        // The state a resolve starts from: both projected, so the claimless one reads as 0.0.
        let store = crate::knowledge::Knowledge::new(&db);
        store.update_projection(resolved_before).await.unwrap();
        store.update_projection(earns_now).await.unwrap();

        let claim = Raw {
            outcome: Outcome::Found,
            payload: Some(json!({"claim": "Q42"})),
        };
        ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers: HashMap::from([("Train Dreams".to_string(), claim)]),
                calls: Arc::new(AtomicUsize::new(0)),
            }))
            .run()
            .await
            .expect("resolve");

        // The passes are separate commands now, so the ordering guarantee is exercised the way the
        // CLI runs it: resolve projects, then scan raises the pair, then merge folds it.
        crate::knowledge::proximity::scan(&store).await.expect("scan");
        let folded = crate::knowledge::proximity::merge(&store).await.expect("merge");
        assert_eq!(folded.merged, 1, "{folded:?}");
        let survivor = model::knowledge_entity::Entity::find_by_id(earns_now)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(
            survivor.merged_into_id.is_none(),
            "the side with two inputs survives: {survivor:?}"
        );
        let folded = model::knowledge_entity::Entity::find_by_id(resolved_before)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            folded.merged_into_id,
            Some(earns_now),
            "the single-input side is the one folded in"
        );
    }

    /// Two entities resolving to the same QID are a *candidate* for one thing, not a verdict: an
    /// album's article names the album and covers each of its songs. Names that disagree stay two
    /// entities, both holding the identity, and `merge` leaves them alone.
    #[tokio::test]
    async fn two_entities_may_resolve_to_one_identity() {
        db_or_skip!(db);
        seed(&db, "Alpha One").await;
        seed(&db, "Beta Two").await;

        let claim = Raw {
            outcome: Outcome::Found,
            payload: Some(json!({"claim": "Q42"})),
        };
        let answers = HashMap::from([
            ("Alpha One".to_string(), claim.clone()),
            ("Beta Two".to_string(), claim),
        ]);

        let report = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers,
                calls: Arc::new(AtomicUsize::new(0)),
            }))
            .run()
            .await
            .expect("resolve");

        assert_eq!(report.claims, 2, "{report:?}");

        // The proximity passes now run after resolve as separate commands; drive them explicitly so
        // the assertion is about `merge` refusing a pair, not about an empty batch.
        let store = crate::knowledge::Knowledge::new(&db);
        crate::knowledge::proximity::scan(&store).await.expect("scan");
        let folded = crate::knowledge::proximity::merge(&store).await.expect("merge");
        assert_eq!(folded.merged, 0, "sharing an identity does not merge");
        let live = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(live, 2, "both entities survive");

        let identity = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::ExternalId.eq("Q42"))
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the identity exists");
        assert_eq!(
            model::knowledge_claims::Entity::find()
                .filter(model::knowledge_claims::Column::KnowledgeIdentityId.eq(identity.id))
                .count(&db.conn)
                .await
                .unwrap(),
            2,
            "one identity, two holders"
        );
    }

    fn request(kind: &str, shape: &str, query: &str) -> Request {
        Request {
            provider: "test",
            kind: kind.to_string(),
            shape: shape.to_string(),
            query: query.to_string(),
        }
    }

    /// `title` and `label` ask about the same string and the answers are different things, so the
    /// group key includes the kind — keyed by query alone the label group once swallowed the titles.
    /// A duplicate within a group is asked once.
    #[test]
    fn plan_groups_key_by_kind_and_collapse_duplicates() {
        let requests = [
            request("label", "v1", "Train Dreams"),
            request("label", "v1", "Train Dreams"),
            request("title", "v1", "Train Dreams"),
            request("search", "v1", "b"),
            request("search", "v1", "a"),
            request("search", "v1", "b"),
        ];
        let groups = plan_groups(&requests);

        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[&("test", "label".into(), "v1".into())],
            vec!["Train Dreams"]
        );
        assert_eq!(
            groups[&("test", "title".into(), "v1".into())],
            vec!["Train Dreams"]
        );
        assert_eq!(groups[&("test", "search".into(), "v1".into())], vec!["a", "b"]);
    }

    /// A spent budget stops issuing calls and says so, rather than silently returning nothing.
    #[tokio::test]
    async fn a_spent_budget_issues_no_calls() {
        db_or_skip!(db);
        let cache = Cache::new(&db.conn);
        let options = Options {
            max_requests: 0,
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let ctx = ResolutionContext::new(&cache, &options);

        ctx.resolve_requests(&Limited, &[request("label", "v1", "Train Dreams")])
            .await;

        assert!(ctx.budget_exhausted(), "the refusal is recorded");
        assert_eq!(
            ctx.fetched.load(Ordering::Relaxed),
            0,
            "and no transport call was made"
        );
    }
}
