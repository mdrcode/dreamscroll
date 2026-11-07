//! The engine: it selects candidates, gives each resolver a context, and stores what comes back.
//!
//! `Gateway` is the only door to the world. A resolver cannot fetch, only resolve
//! requests; the context consults the cache first, paces the call and charges the budget.

use std::collections::HashSet;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use super::cache::Cache;
use super::contract::*;
use super::gateway::Gateway;
use super::store::store_claim;
use crate::{database, model};

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
    pub planned: usize,
    pub cache_hits: usize,
    /// Transport calls made.
    pub fetched: usize,
    pub failed: usize,
    pub claims: usize,
    /// Entities that gained at least one claim.
    pub entities_resolved: usize,
    /// Claims an editor's veto kept out of the graph.
    pub vetoed: usize,
    pub budget_exhausted: bool,
    /// Set when the provider asked us to back off: the run stopped instead of risking a ban.
    pub rate_limited_after: Option<u64>,
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
        let cache = Cache::new(&self.db.conn);
        let gateway = Gateway::new(&cache, &self.options);
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

        for resolver in &self.resolvers {
            if gateway.rate_limited_after().is_some() {
                break;
            }
            // One entity, one identity: the first claim the graph accepts wins, and the rest of that
            // entity's claims are dropped. The resolver's ordering is its own priority.
            for (knowledge_entity_id, claim) in resolver.resolve(&entities, &gateway).await {
                if resolved.contains(&knowledge_entity_id) {
                    continue;
                }
                if store_claim(
                    self.db,
                    resolver.id(),
                    knowledge_entity_id,
                    claim,
                    &vetoes,
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
        let stats = gateway.stats();
        report.planned = stats.planned;
        report.cache_hits = stats.cache_hits;
        report.fetched = stats.fetched;
        report.failed = stats.failed;
        report.budget_exhausted = gateway.budget_exhausted();
        report.rate_limited_after = gateway.rate_limited_after();
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
                model::knowledge_identity::Column::Source,
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

    use crate::knowledge::identity::cache::Outcome;
    use sea_orm::PaginatorTrait;
    use serde_json::json;
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

    /// An authority that never touches the network: it answers from a fixed map and counts calls.
    struct Fake {
        answers: HashMap<String, Raw>,
        calls: Arc<AtomicUsize>,
    }

    fn fake_request(entity: &model::knowledge_entity::Model) -> Request {
        Request {
            provider: "fake",
            kind: Kind::Label,
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
            let requests: Vec<Request> = entities.iter().map(fake_request).collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&(
                        "fake".to_string(),
                        Kind::Label,
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
            gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request> = entities.iter().map(fake_request).collect();
            gateway.resolve_requests(self, &requests).await;
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
            gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            let requests: Vec<Request> = entities
                .iter()
                .map(|entity| Request {
                    provider: "flaky",
                    kind: Kind::Label,
                    shape: "v1".to_string(),
                    query: entity.name.clone(),
                })
                .collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&(
                        "flaky".to_string(),
                        Kind::Label,
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

    fn request(kind: Kind, shape: &str, query: &str) -> Request {
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
        let cache = Cache::new(&db.conn);
        let options = Options {
            max_requests: 0,
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let gateway = Gateway::new(&cache, &options);

        gateway
            .resolve_requests(&Limited, &[request(Kind::Label, "v1", "Train Dreams")])
            .await;

        assert!(gateway.budget_exhausted(), "the refusal is recorded");
        assert_eq!(gateway.fetched(), 0, "and no transport call was made");
    }
}
