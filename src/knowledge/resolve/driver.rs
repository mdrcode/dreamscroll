//! What a resolver is, and the engine that runs it.
//!
//! A `Resolver` is one authority plus the strategy for getting claims out of it: it is handed a
//! batch of entities and returns claims about them. How it gets there — one request or a ladder of
//! them, one pass or several — is its own business.
//!
//! Adding an authority means writing one `Resolver`. It cannot forget the cache, because
//! `ResolutionContext::ask` is the only way to reach the world: there is no other door.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::{Value, json};

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
    /// Entities folded into the entity that already held their identity.
    pub merges: usize,
    /// Identity conflicts that survived the same-capture veto — real contradictions, kept as
    /// collisions for adjudication rather than folded.
    pub collisions: usize,
    pub budget_exhausted: bool,
    /// Set when the provider asked us to back off: the run stopped instead of risking a ban.
    pub rate_limited_after: Option<u64>,
    pub offline: bool,
}

/// No rate limit seen. Distinguishes "the provider did not ask us to wait" from "wait zero seconds".
const NOT_LIMITED: u64 = u64::MAX;

/// What a resolver is given.
///
/// It is the only door to the world: a resolver cannot fetch, only ask. That is what lets one own its
/// own strategy — including several passes — without being able to bypass the cache, the pace or the
/// budget.
pub struct ResolutionContext<'a> {
    cache: &'a Cache<'a>,
    pacer: tokio::sync::Mutex<Pacer>,
    budget: tokio::sync::Mutex<Budget>,
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
            budget: tokio::sync::Mutex::new(Budget::new(options.max_requests)),
            offline: options.offline,
            planned: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            fetched: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            budget_exhausted: AtomicBool::new(false),
            rate_limited_after: AtomicU64::new(NOT_LIMITED),
        }
    }

    /// Whether the run is prohibited from touching the network. A resolver that plans several passes
    /// can stop after the first: with nothing fetched, a second pass has nothing new to say.
    pub fn offline(&self) -> bool {
        self.offline
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
    pub async fn ask<T: Transport + ?Sized>(&self, transport: &T, requests: &[Request]) -> Answers {
        let mut answers = Answers::new();
        if requests.is_empty() {
            return answers;
        }
        self.planned.fetch_add(requests.len(), Ordering::Relaxed);

        for ((provider, kind, shape), queries) in plan_groups(requests) {
            let hits = match self.cache.get_many(provider, &kind, &shape, &queries).await {
                Ok(hits) => hits,
                Err(error) => {
                    tracing::warn!(%error, "could not read the cache");
                    continue;
                }
            };
            self.cache_hits.fetch_add(hits.len(), Ordering::Relaxed);
            for (query, answer) in hits {
                answers.insert(
                    (kind.clone(), shape.clone(), query),
                    Raw {
                        outcome: answer.outcome,
                        payload: answer.payload,
                    },
                );
            }

            let misses: Vec<String> = queries
                .into_iter()
                .filter(|query| {
                    !answers.contains_key(&(kind.clone(), shape.clone(), query.clone()))
                })
                .collect();
            if misses.is_empty() || self.offline {
                continue;
            }

            for chunk in misses.chunks(transport.batch_size(&kind)) {
                if !self.budget.lock().await.take() {
                    self.budget_exhausted.store(true, Ordering::Relaxed);
                    break;
                }
                let batch: Vec<Request> = chunk
                    .iter()
                    .map(|query| Request {
                        provider,
                        kind: kind.clone(),
                        shape: shape.clone(),
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
                                    &kind,
                                    &shape,
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
                            answers.insert((kind.clone(), shape.clone(), query), raw);
                        }
                    }
                    Err(FetchError::RateLimited { retry_after_secs }) => {
                        // An instruction, not a fact. Do not cache it, do not retry, do not
                        // continue: WDQS temporarily bans clients that keep requesting after a 429.
                        self.rate_limited_after
                            .store(retry_after_secs, Ordering::Relaxed);
                        return answers;
                    }
                    Err(FetchError::Failed(_)) => {
                        // Not a fact about the world either: cache it briefly so a sick provider is
                        // not hammered, then move on.
                        for query in chunk {
                            if let Err(error) = self
                                .cache
                                .put(
                                    provider,
                                    &kind,
                                    &shape,
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
                                (kind.clone(), shape.clone(), query.clone()),
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
        }

        answers
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
                    &knowledge,
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

        for knowledge_entity_id in &resolved {
            knowledge.update_projection(*knowledge_entity_id).await?;
        }
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

/// How many transport calls this run may still make.
struct Budget {
    remaining: usize,
    spent: bool,
}

impl Budget {
    fn new(max_requests: usize) -> Self {
        Self {
            remaining: max_requests,
            spent: false,
        }
    }

    /// Take one call. Returns false when the budget is gone, having recorded that it is.
    fn take(&mut self) -> bool {
        if self.remaining == 0 {
            self.spent = true;
            return false;
        }
        self.remaining -= 1;
        true
    }
}

/// Store one claim as `ontology` evidence. Returns whether it was accepted; a claim another entity
/// already holds is a conflict (a merge the graph has not made), not a duplicate.
async fn store_claim(
    db: &database::DbHandle,
    knowledge: &crate::knowledge::Knowledge,
    source: &str,
    knowledge_entity_id: i32,
    claim: Claim,
    report: &mut Report,
) -> anyhow::Result<bool> {
    let (scheme, identity) = (claim.scheme.clone(), claim.id.clone());
    let value = json!({
        "scheme": claim.scheme,
        "id": claim.id,
        "label": claim.label,
        "description": claim.description,
        "classes": claim.classes,
    });
    let insert =
        model::knowledge_evidence::Entity::insert(model::knowledge_evidence::ActiveModel {
            knowledge_entity_id: Set(knowledge_entity_id),
            kind: Set("claim".to_string()),
            value: Set(value),
            source: Set(source.to_string()),
            method: Set(Some(claim.method)),
            confidence: Set(Some(claim.confidence)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await;

    match insert {
        Ok(_) => {
            report.claims += 1;
            Ok(true)
        }
        Err(error) => match crate::knowledge::evidence::unique_violation(&error) {
            // Another entity already holds this identity, which means the two entities are the same
            // referent: the graph was told so by the authority. Fold ours into the one that holds it
            // rather than leaving a duplicate behind.
            Some("idx-knowledge_evidence_identity") => {
                let claim = crate::knowledge::evidence::Claim {
                    scheme: scheme.clone(),
                    id: identity.clone(),
                    handle: None,
                };
                if let Some(owner) = crate::knowledge::evidence::Evidence::new(db.conn.clone())
                    .owner(&claim)
                    .await?
                    && owner != knowledge_entity_id
                {
                    match knowledge.merge(knowledge_entity_id, owner).await? {
                        crate::knowledge::merge::Outcome::Merged => report.merges += 1,
                        crate::knowledge::merge::Outcome::AlreadyMerged => {}
                        // The veto says these two cannot be one referent, so the authority is
                        // contradicting something we already know. Record it; do not fold.
                        crate::knowledge::merge::Outcome::RefusedSharedIllumination => {
                            // Recorded once per pair. A resolve run retries every entity without a
                            // claim, so re-asserting the same contradiction would otherwise append
                            // a row per run and turn "3 collisions" into noise.
                            if !collision_recorded(&db.conn, owner, knowledge_entity_id, &identity)
                                .await?
                            {
                                report.collisions += 1;
                                model::knowledge_evidence::Entity::insert(
                                    model::knowledge_evidence::ActiveModel {
                                        knowledge_entity_id: Set(owner),
                                        kind: Set("collision".to_string()),
                                        value: Set(json!({
                                            "reason": "shared illumination",
                                            "claimant": knowledge_entity_id,
                                            "scheme": scheme,
                                            "id": identity,
                                        })),
                                        source: Set("model".to_string()),
                                        ..Default::default()
                                    },
                                )
                                .exec(&db.conn)
                                .await?;
                            }
                        }
                    }
                }
                Ok(false)
            }
            // The same fact already recorded for this entity: nothing to do.
            Some(_) => Ok(false),
            None => Err(error.into()),
        },
    }
}

/// Whether this pair already carries a live collision for the same identity.
async fn collision_recorded(
    conn: &sea_orm::DatabaseConnection,
    owner: i32,
    other: i32,
    identity: &str,
) -> anyhow::Result<bool> {
    let row = conn
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) FROM knowledge_evidence WHERE kind = 'collision' \
             AND retracted_at IS NULL AND knowledge_entity_id = $1 AND (value->>'claimant')::int = $2 \
             AND value->>'id' = $3",
            [owner.into(), other.into(), identity.into()],
        ))
        .await?;
    let count: i64 = row
        .map(|row| row.try_get_by_index(0))
        .transpose()?
        .unwrap_or(0);
    Ok(count > 0)
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
    use model::knowledge_evidence as evidence;

    let already: HashSet<i32> = evidence::Entity::find()
        .filter(evidence::Column::Kind.eq("claim"))
        .filter(evidence::Column::Source.ne("platform"))
        .filter(evidence::Column::RetractedAt.is_null())
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.knowledge_entity_id)
        .collect();

    let mut entities = entity::Entity::find()
        .filter(entity::Column::MergedIntoId.is_null())
        .order_by_asc(entity::Column::Id)
        .all(conn)
        .await?;
    entities.retain(|row| !already.contains(&row.id));
    if let Some(knowledge_entity_id) = only {
        entities.retain(|row| row.id == knowledge_entity_id);
    }
    if let Some(limit) = limit {
        entities.truncate(limit);
    }

    Ok(entities)
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;

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
        fn id(&self) -> &'static str {
            "fake"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            ctx: &ResolutionContext<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request> = entities.iter().map(fake_request).collect();
            let answers = ctx.ask(self, &requests).await;
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
                            scheme: "fake".to_string(),
                            id: id.to_string(),
                            label: Some(entity.name.clone()),
                            description: None,
                            classes: Vec::new(),
                            method: "exact_label".to_string(),
                            confidence: 0.9,
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
        use crate::test_support::fixtures;

        let user_id = fixtures::user(db, "resolve").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let entity_id =
            model::user_observed_entity::Entity::insert(model::user_observed_entity::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(0),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap()
            .last_insert_id as i32;

        crate::test_support::link::link_all(db).await.unwrap();

        model::user_observed_entity::Entity::find_by_id(entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .knowledge_entity_id
            .expect("build_core should have linked the observation")
    }

    #[tokio::test]
    async fn a_claim_identifies_the_core_and_a_second_run_asks_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
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
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
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
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let knowledge_entity_id = seed(&db, "Nintendo").await;

        model::knowledge_evidence::Entity::insert(model::knowledge_evidence::ActiveModel {
            knowledge_entity_id: Set(knowledge_entity_id),
            kind: Set("claim".to_string()),
            value: Set(json!({"scheme": "wikidata", "id": "Q8093"})),
            source: Set("wikidata".to_string()),
            method: Set(Some("exact_label".to_string())),
            confidence: Set(Some(0.8)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
        model::knowledge_evidence::Entity::insert(model::knowledge_evidence::ActiveModel {
            knowledge_entity_id: Set(knowledge_entity_id),
            kind: Set("claim".to_string()),
            value: Set(json!({"scheme": "x_twitter", "id": "nintendo"})),
            source: Set("platform".to_string()),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        crate::knowledge::Knowledge::with_linker(
            &db,
            std::sync::Arc::new(crate::knowledge::heuristic::HeuristicLinker),
            Box::new(
                crate::knowledge::heuristic::NameBlocker::new(&db)
                    .await
                    .unwrap(),
            ),
        )
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

    /// A `p2002` claim *is* the handle — it must not count as a second input beside it.
    #[tokio::test]
    async fn a_handle_derived_claim_is_not_corroboration() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let knowledge_entity_id = seed(&db, "Tsoding").await;

        for (source, method, value, confidence) in [
            (
                "platform",
                None,
                json!({"scheme": "x_twitter", "id": "tsoding"}),
                None,
            ),
            (
                "wikidata",
                Some("p2002"),
                json!({"scheme": "wikidata", "id": "Q123737256"}),
                Some(0.95),
            ),
        ] {
            model::knowledge_evidence::Entity::insert(model::knowledge_evidence::ActiveModel {
                knowledge_entity_id: Set(knowledge_entity_id),
                kind: Set("claim".to_string()),
                value: Set(value),
                source: Set(source.to_string()),
                method: Set(method.map(str::to_string)),
                confidence: Set(confidence),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap();
        }

        crate::knowledge::Knowledge::with_linker(
            &db,
            std::sync::Arc::new(crate::knowledge::heuristic::HeuristicLinker),
            Box::new(
                crate::knowledge::heuristic::NameBlocker::new(&db)
                    .await
                    .unwrap(),
            ),
        )
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
            ctx.ask(self, &requests).await;
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
            let answers = ctx.ask(self, &requests).await;
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
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
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
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
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

    /// Two different entities resolving to the same QID are the same referent. The second must fold
    /// into the first rather than leaving a duplicate behind.
    #[tokio::test]
    async fn cores_that_resolve_to_the_same_identity_are_merged() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db, "Alpha One").await;
        seed(&db, "Beta Two").await;
        let before = model::knowledge_entity::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(before, 2);

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

        assert_eq!(report.merges, 1, "{report:?}");
        let live = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(live, 1, "one entity survives");
        let tombstoned = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::MergedIntoId.is_not_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(tombstoned, 1, "the loser is a tombstone, not a deletion");

        let survivor = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let pointing = model::user_observed_entity::Entity::find()
            .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(survivor.id))
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(pointing, 2, "both observations follow the survivor");
    }

    #[tokio::test]
    async fn offline_plans_without_fetching() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db, "Train Dreams").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let options = Options {
            offline: true,
            ..Default::default()
        };

        let report = ResolutionEngine::new(&db, options)
            .with(Box::new(Fake {
                answers: HashMap::new(),
                calls: calls.clone(),
            }))
            .run()
            .await
            .expect("resolve");

        assert_eq!(report.planned, 1, "{report:?}");
        assert_eq!(report.fetched, 0, "{report:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
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
    #[test]
    fn plans_group_by_kind_and_shape() {
        let requests = [
            request("label", "v1", "Train Dreams"),
            request("label", "v1", "Train Dreams"),
            request("title", "v1", "Train Dreams"),
        ];
        let groups = plan_groups(&requests);

        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[&("test", "label".into(), "v1".into())],
            vec!["Train Dreams"]
        );
        assert_eq!(
            groups[&("test", "title".into(), "v1".into())],
            vec!["Train Dreams"]
        );
    }

    #[test]
    fn duplicates_within_a_group_are_collapsed() {
        let requests = [
            request("search", "v1", "b"),
            request("search", "v1", "a"),
            request("search", "v1", "b"),
        ];
        let groups = plan_groups(&requests);
        assert_eq!(
            groups[&("test", "search".into(), "v1".into())],
            vec!["a", "b"]
        );
    }

    #[test]
    fn the_budget_stops_when_spent_and_says_so() {
        let mut budget = Budget::new(2);
        assert!(budget.take());
        assert!(budget.take());
        assert!(!budget.take(), "the third call is refused");
        assert!(budget.spent, "and the refusal is recorded");
    }

    struct AlwaysRateLimited;

    #[async_trait::async_trait]
    impl Transport for AlwaysRateLimited {
        async fn fetch(&self, _requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            Err(FetchError::RateLimited {
                retry_after_secs: 7,
            })
        }
    }

    /// A 429 is an instruction: stop, cache nothing, report the wait. Caching it would turn a
    /// temporary throttle into a week-long "no such entity".
    #[tokio::test]
    async fn a_rate_limit_stops_the_ask_and_caches_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let cache = Cache::new(&db.conn);
        let options = Options {
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let ctx = ResolutionContext::new(&cache, &options);

        let answers = ctx
            .ask(
                &AlwaysRateLimited,
                &[request("label", "v1", "Train Dreams")],
            )
            .await;

        assert!(
            answers.is_empty(),
            "a 429 must not be recorded as an answer"
        );
        assert_eq!(ctx.rate_limited_after(), Some(7));
        assert_eq!(
            ctx.fetched.load(Ordering::Relaxed),
            1,
            "it stopped after the first call"
        );
    }

    /// A spent budget stops issuing calls and says so, rather than silently returning nothing.
    #[tokio::test]
    async fn a_spent_budget_issues_no_calls() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let cache = Cache::new(&db.conn);
        let options = Options {
            max_requests: 0,
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let ctx = ResolutionContext::new(&cache, &options);

        ctx.ask(
            &AlwaysRateLimited,
            &[request("label", "v1", "Train Dreams")],
        )
        .await;

        assert!(ctx.budget_exhausted(), "the refusal is recorded");
        assert_eq!(
            ctx.fetched.load(Ordering::Relaxed),
            0,
            "and no transport call was made"
        );
    }
}
