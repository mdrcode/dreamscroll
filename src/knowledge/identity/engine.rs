//! The engine: it selects candidates, gives each resolver a context, and stores what comes back.
//!
//! `Gateway` is the only door to the world. A resolver cannot fetch, only resolve
//! requests; the context consults the cache first, paces the call and charges the budget.

use std::collections::HashSet;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use super::resolver::Resolver;
use crate::fetch::{Allowance, FetchCache, Gateway, Refusal};
use crate::knowledge::evidence::{Stored, store_claim};
use crate::{database, model};

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
    /// Consider the newest entities first.
    ///
    /// The budget is finite, so *which* candidates it reaches decides what a short run achieves.
    /// Ascending id is right for a rebuild (deterministic, and the oldest entities have waited
    /// longest); newest-first is right when captures just arrived, because otherwise a run spends
    /// its 200 requests on candidates that have failed every previous pass.
    pub newest_first: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            limit: None,
            max_requests: DEFAULT_MAX_REQUESTS,
            min_interval: std::time::Duration::from_secs(1),
            offline: false,
            only: None,
            newest_first: false,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub candidates: usize,
    /// What the run asked, what the cache answered, and what it fetched.
    pub stats: crate::fetch::Stats,
    pub claims: usize,
    /// Entities that gained at least one claim.
    pub entities_resolved: usize,
    /// Claims an editor's veto kept out of the graph.
    pub vetoed: usize,
    /// Why the run stopped early, if it did: the budget, or the provider's `429`. The caller prints
    /// what it needs from it, instead of this report carrying three derived answers to one question.
    pub refusal: Option<Refusal>,
    pub offline: bool,
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
        // An entity is a candidate for a provider that has not identified it yet. Asking "any
        // authority" instead would stop Wikidata resolving an entity the moment IMDb had.
        let sources: Vec<&str> = self
            .resolvers
            .iter()
            .map(|resolver| resolver.id())
            .collect();
        let entities = candidates(
            &self.db.conn,
            self.options.limit,
            self.options.only,
            self.options.newest_first,
            &sources,
        )
        .await?;
        let knowledge = crate::knowledge::Knowledge::new(self.db);
        // Loaded once, not per claim: a veto asked during admission is a claim never written; one
        // that is only replayed is a claim asserted and then withdrawn.
        let vetoes = crate::knowledge::verdicts::vetoed(&knowledge).await?;
        let cache = FetchCache::new(&self.db.conn);
        // The engine owns *which* entities to ask about. What the run may spend is the allowance's,
        // passed in; the gateway only asks it before each call. A cache-only run has none at all.
        let allowance = (!self.options.offline).then(|| {
            Allowance::new(self.options.max_requests, self.options.min_interval)
        });
        let gateway = Gateway::new(Some(&cache), allowance.as_ref(), true);
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
                tracing::info!(pruned, "Pruned expired fetch cache rows");
            }
        }

        let mut resolved: HashSet<i32> = HashSet::new();

        for resolver in &self.resolvers {
            // A provider that told us to stop gets no further questions.
            if matches!(
                allowance.as_ref().and_then(Allowance::refused),
                Some(Refusal::RateLimited { .. })
            ) {
                break;
            }
            // One entity, one identity: the first claim the graph accepts wins, and the rest of that
            // entity's claims are dropped. The resolver's ordering is its own priority.
            for (knowledge_entity_id, claim) in resolver.resolve(&entities, &gateway).await {
                if resolved.contains(&knowledge_entity_id) {
                    continue;
                }
                match store_claim(
                    self.db,
                    resolver.id(),
                    knowledge_entity_id,
                    claim,
                    &vetoes,
                )
                .await?
                {
                    Stored::Written => {
                        report.claims += 1;
                        resolved.insert(knowledge_entity_id);
                    }
                    Stored::Vetoed => report.vetoed += 1,
                    // The entity already asserts it; the first claim still wins.
                    Stored::AlreadyClaimed => {}
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
        report.stats = gateway.stats();
        report.refusal = allowance.as_ref().and_then(Allowance::refused);
        Ok(report)
    }
}

/// Entities with no live ontology claim. Note the filter is *ontology*, not *no evidence*: an entity
/// identified only by a platform handle still deserves a QID, because that is how corroboration
/// happens.
async fn candidates(
    conn: &sea_orm::DatabaseConnection,
    limit: Option<usize>,
    only: Option<i32>,
    newest_first: bool,
    sources: &[&str],
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
                model::knowledge_identity::Column::AssertedBy,
            ))
            .is_in(
                sources
                    .iter()
                    .map(|source| source.to_string())
                    .collect::<Vec<_>>(),
            ),
        )
        .to_owned();

    let mut query = entity::Entity::find()
        .filter(entity::Column::MergedIntoId.is_null())
        .filter(Expr::col(entity::Column::Id).not_in_subquery(claimed))
        // Which candidates a finite budget reaches is a policy, not a detail: ascending is the
        // rebuild order, descending is "what just arrived".
        .order_by(
            entity::Column::Id,
            if newest_first {
                sea_orm::Order::Desc
            } else {
                sea_orm::Order::Asc
            },
        );
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
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

    use crate::fetch::{Answer, BATCH, FetchError, Outcome, Request, Transport};
    use crate::knowledge::evidence::Claim;
    use sea_orm::PaginatorTrait;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::test_support::db_or_skip;

    /// Which candidates a finite budget reaches is a policy, so pin both orders.
    #[tokio::test]
    async fn candidates_are_ordered_by_the_run_not_by_habit() {
        db_or_skip!(db);
        let first = crate::test_support::fixtures::knowledge_entity_with_member(&db, "First").await;
        let second =
            crate::test_support::fixtures::knowledge_entity_with_member(&db, "Second").await;

        let ascending = candidates(&db.conn, Some(2), None, false, &["wikidata"])
            .await
            .unwrap();
        let descending = candidates(&db.conn, Some(2), None, true, &["wikidata"])
            .await
            .unwrap();

        assert_eq!(
            ascending.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![first, second],
            "a rebuild walks from the oldest"
        );
        assert_eq!(
            descending.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![second, first],
            "a run that serves fresh captures starts at the newest"
        );
    }

    /// The engine is provider-agnostic: this fake brings its own kind and payload, and neither is
    /// anything an authority would recognise.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    enum Probe {
        Text,
    }

    impl std::fmt::Display for Probe {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("text")
        }
    }

    /// An authority that never touches the network: it answers from a fixed map and counts calls.
    /// An answer that identified one item, for the fakes below.
    fn identified(id: &str) -> Answer<Vec<String>> {
        Answer {
            outcome: Outcome::Found,
            body: Some(vec![id.to_string()]),
        }
    }

    struct Fake {
        answers: HashMap<String, Answer<Vec<String>>>,
        calls: Arc<AtomicUsize>,
    }

    fn fake_request(entity: &model::knowledge_entity::Model) -> Request<Probe> {
        Request {
            provider: "fake",
            kind: Probe::Text,
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
            gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request<Probe>> = entities.iter().map(fake_request).collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&fake_request(entity))?;
                    let id = raw.body.as_ref()?.first()?.clone();
                    Some((
                        entity.id,
                        Claim {
                            scheme: "wikidata".to_string(),
                            id,
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
        type Kind = Probe;
        type Payload = Vec<String>;

        async fn fetch(
            &self,
            requests: &[Request<Probe>],
        ) -> Result<Vec<(String, Answer<Vec<String>>)>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(requests
                .iter()
                .map(|request| {
                    let answer = self
                        .answers
                        .get(&request.query)
                        .cloned()
                        .unwrap_or(Answer::nothing(Outcome::Empty));
                    (request.query.clone(), answer)
                })
                .collect())
        }
    }

    /// One observation, linked to an entity by the real linker — the same path production takes.
    async fn seed(db: &database::DbHandle, name: &str) -> i32 {
        let capture = crate::test_support::fixtures::Capture::new(db, "resolve").await;
        let observation =
            crate::test_support::fixtures::observe(db, &capture, 0, name, None, "book", None, None)
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
            identified("Q185971"),
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
        assert_eq!(first.stats.fetched, 1, "{first:?}");
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
        assert_eq!(first.stats.fetched, 1, "{first:?}");
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
        assert_eq!(second.stats.fetched, 0, "{second:?}");
        assert_eq!(second.stats.cache_hits, 1, "{second:?}");
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

    /// The local-development workflow: build the graph from answers already on disk, with no network
    /// at all. This is how `resolve` is run locally every time, so it is pinned end to end — zero
    /// transport calls, and the claims still land.
    #[tokio::test]
    async fn an_offline_run_builds_the_graph_from_the_cache_alone() {
        db_or_skip!(db);
        let cached = seed(&db, "Train Dreams").await;
        // A second candidate with nothing cached: offline must leave it unresolved rather than fetch.
        let uncached = seed(&db, "MUME").await;

        // The answer a previous online run would have left behind.
        FetchCache::new(&db.conn)
            .put(
                "fake",
                &Probe::Text,
                "v1",
                "Train Dreams",
                &identified("Q185971"),
            )
            .await
            .expect("seed the cache");

        let calls = Arc::new(AtomicUsize::new(0));
        let report = ResolutionEngine::new(
            &db,
            Options {
                offline: true,
                ..Default::default()
            },
        )
        .with(Box::new(Fake {
            answers: HashMap::new(),
            calls: calls.clone(),
        }))
        .run()
        .await
        .expect("resolve");

        assert!(report.offline, "the run says it was cache-only");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "offline never touches the transport"
        );
        assert_eq!(report.stats.fetched, 0);
        assert_eq!(report.stats.cache_hits, 1);
        assert_eq!(report.claims, 1, "only the cached one resolves: {report:?}");

        let entity = model::knowledge_entity::Entity::find_by_id(cached)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entity.status, "identified", "{entity:?}");

        // The miss is left alone, which is the whole point: offline builds what the cache can and
        // stops there.
        let entity = model::knowledge_entity::Entity::find_by_id(uncached)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entity.status, "provisional", "{entity:?}");
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
            gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request<Probe>> = entities.iter().map(fake_request).collect();
            gateway.resolve_requests(self, &requests).await;
            Vec::new()
        }
    }

    #[async_trait::async_trait]
    impl Transport for Limited {
        type Kind = Probe;
        type Payload = Vec<String>;

        async fn fetch(
            &self,
            _requests: &[Request<Probe>],
        ) -> Result<Vec<(String, Answer<Vec<String>>)>, FetchError> {
            Err(FetchError::RateLimited {
                retry_after_secs: Some(30),
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
            gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request<Probe>> = entities
                .iter()
                .map(|entity| Request {
                    provider: "flaky",
                    kind: Probe::Text,
                    shape: "v1".to_string(),
                    query: entity.name.clone(),
                })
                .collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&Request {
                        provider: "flaky",
                        kind: Probe::Text,
                        shape: "v1".to_string(),
                        query: entity.name.clone(),
                    })?;
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
        type Kind = Probe;
        type Payload = Vec<String>;

        async fn fetch(
            &self,
            requests: &[Request<Probe>],
        ) -> Result<Vec<(String, Answer<Vec<String>>)>, FetchError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) >= 1 {
                return Err(FetchError::RateLimited {
                    retry_after_secs: Some(60),
                });
            }
            Ok(requests
                .iter()
                .map(|request| {
                    (
                        request.query.clone(),
                        Answer {
                            outcome: Outcome::Found,
                            body: Some(Vec::new()),
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

        assert_eq!(
            report.refusal,
            Some(Refusal::RateLimited {
                retry_after_secs: Some(60)
            }),
            "{report:?}"
        );
        assert_eq!(
            report.stats.fetched, 2,
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

        assert_eq!(
            report.refusal,
            Some(Refusal::RateLimited {
                retry_after_secs: Some(30)
            }),
            "{report:?}"
        );
        assert_eq!(report.stats.fetched, 1, "{report:?}");
        assert_eq!(report.claims, 0, "{report:?}");
        assert_eq!(
            model::fetch_cache::Entity::find()
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

        let claim = identified("Q42");
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
        crate::knowledge::proximity::scan(&store)
            .await
            .expect("scan");
        let folded = crate::knowledge::proximity::merge(&store)
            .await
            .expect("merge");
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

        let claim = identified("Q42");
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
        crate::knowledge::proximity::scan(&store)
            .await
            .expect("scan");
        let folded = crate::knowledge::proximity::merge(&store)
            .await
            .expect("merge");
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

    fn request(kind: Probe, shape: &str, query: &str) -> Request<Probe> {
        Request {
            provider: "test",
            kind,
            shape: shape.to_string(),
            query: query.to_string(),
        }
    }

    #[tokio::test]
    async fn a_spent_budget_issues_no_calls() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let allowance = Allowance::new(0, std::time::Duration::ZERO);
        let gateway = Gateway::new(Some(&cache), Some(&allowance), true);

        gateway
            .resolve_requests(&Limited, &[request(Probe::Text, "v1", "Train Dreams")])
            .await;

        assert_eq!(
            allowance.refused(),
            Some(Refusal::Budget),
            "the refusal is recorded"
        );
        assert_eq!(gateway.fetched(), 0, "and no transport call was made");
    }
}
