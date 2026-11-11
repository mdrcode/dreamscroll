//! Resolution: the authority contract, and the run loop that drives it.
//!
//! A resolver owns its strategy — one request or a ladder of them, one pass or several. `Gateway` is
//! its only door to the world: the gateway consults the cache first, paces the call and charges the
//! budget. The engine selects the entities, gives each resolver a batch, and stores what comes back.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::RESERVED_PROVIDERS;
use crate::fetch::{Allowance, FetchCache, Gateway, Refusal};
use crate::knowledge::KnowledgeGraph;
use crate::knowledge::evidence::{Claim, ClaimDraft, DraftIdentity, Evidence, Stored};
use crate::model;

/// Transport calls a plain resolve is allowed to make when the caller does not choose a budget.
/// Exists so that omitting the budget does real work instead of quietly resolving nothing.
pub const DEFAULT_MAX_REQUESTS: usize = 200;

/// Claims one resolver may assert for one entity in a run. An agentic authority must not inflate a
/// confidence by returning a hundred URLs.
const MAX_CLAIMS_PER_ENTITY: usize = 8;

/// One authority, and the strategy for asking it.
///
/// `resolve` takes a *batch* of entities rather than one: batching across entities is the difference
/// between a hundred transport calls for a corpus walk and five thousand, and only the resolver knows
/// how to group its own requests.
///
/// Request, answer and transport shapes are deliberately *not* here — they belong to [`crate::fetch`],
/// which knows nothing about knowledge entities. This trait knows nothing about the network, the
/// cache or the budget.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    /// The authority's id: the `provider` on every request it makes, the `provider` on every
    /// identity it writes, and what `Resolver::provider` returns. It may not be one of
    /// [`RESERVED_PROVIDERS`]; [`ResolutionEngine::new`] refuses those when the run is built.
    fn provider(&self) -> &'static str;

    /// Claims about the entities it could resolve, each tagged with the entity it belongs to, in the
    /// resolver's priority order. The engine writes every claim the graph accepts: several identities
    /// for one entity are normal, and a later run skips only the entities this provider already
    /// claimed.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Claim)>;
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Ask every resolver even when it already claimed the entity. `resolve --entity` sets this;
    /// without it, the per-provider claimed filter makes an entity-scoped run empty. Already-held
    /// identities are no-ops; new ones are added.
    pub include_already_claimed: bool,
    /// Retract this provider's old claims in the schemes it answered but did not re-propose.
    /// Requires `include_already_claimed`; `resolve --entity --replace` sets both.
    pub replace: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            include_already_claimed: false,
            replace: false,
        }
    }
}

/// One resolution run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolutionReport {
    /// Entities in scope after the tombstone and duplicate filter.
    pub considered: usize,
    /// What the run asked, what the cache answered, and what it fetched.
    pub traffic: crate::fetch::Stats,
    /// Claims the graph accepted.
    pub written: usize,
    /// Distinct entities that gained at least one claim.
    pub entities_resolved: usize,
    /// Claims an editor's veto kept out of the graph.
    pub vetoed: usize,
    /// One entry per provider called, so a failure or a silent provider is attributable.
    pub by_provider: Vec<ProviderReport>,
    /// Why the run stopped early, if it did: the budget, or the provider's `429`.
    pub refusal: Option<Refusal>,
    pub offline: bool,
}

/// What one provider did in one run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProviderReport {
    pub provider: String,
    /// Entities handed to this provider after its own selection.
    pub batch: usize,
    /// Claims this provider got written.
    pub written: usize,
    pub vetoed: usize,
    /// Claims refused by the per-entity cap or returned outside the batch.
    pub dropped: usize,
    /// Old claims this provider owned that the re-resolve did not re-propose.
    pub replaced: usize,
    /// This provider's share of [`ResolutionReport::traffic`].
    pub traffic: crate::fetch::Stats,
}

/// The harness: it holds the resolvers, gives them a context, and stores what they return.
///
/// It knows nothing about stages, tiers or ladders. A resolver that wants two passes takes them; the
/// engine only orders resolvers and merges their claims.

pub struct ResolutionEngine {
    graph: KnowledgeGraph,
    options: Options,
    resolvers: Vec<Box<dyn Resolver>>,
}

impl ResolutionEngine {
    /// Build a run. Both checks are construction invariants: a reserved provider or a replace
    /// without include_already_claimed is a wiring error, not something to rediscover on every
    /// `resolve`.
    pub fn new(
        graph: KnowledgeGraph,
        options: Options,
        resolvers: Vec<Box<dyn Resolver>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !options.replace || options.include_already_claimed,
            "replace requires include_already_claimed"
        );
        for resolver in &resolvers {
            anyhow::ensure!(
                !RESERVED_PROVIDERS.contains(&resolver.provider()),
                "resolver id `{}` is reserved; it names our own pipeline, not an authority",
                resolver.provider()
            );
        }
        Ok(Self {
            graph,
            options,
            resolvers,
        })
    }

    pub(crate) fn providers(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.resolvers.iter().map(|resolver| resolver.provider())
    }

    /// Resolve exactly the entities in scope; selection is the caller's job.
    ///
    /// The helper layer composes the scopes: one illumination, or the pending world. The caller owns
    /// the cache and the allowance; this method builds the ephemeral gateway from them. A shared
    /// `Allowance` therefore caps several runs, while a fresh one gives each run its own budget.
    pub async fn resolve(
        &self,
        entities: &[crate::model::knowledge_entity::Model],
        cache: &FetchCache<'_>,
        allowance: Option<&Allowance>,
    ) -> anyhow::Result<ResolutionReport> {
        // The caller may hand in a tombstone or a duplicate; neither is a question to ask.
        let mut seen = HashSet::new();
        let entities: Vec<crate::model::knowledge_entity::Model> = entities
            .iter()
            .filter(|entity| entity.merged_into_id.is_none() && seen.insert(entity.id))
            .cloned()
            .collect();

        let gateway = Gateway::new(Some(cache), allowance, true);
        let mut run = ResolutionRun::new(self, &entities, &gateway).await?;
        run.collect(&entities).await?;
        run.apply().await?;
        Ok(run.finish())
    }
}

/// The mutable state of one `resolve`: what was asked, what came back, and what was written.
///
/// It borrows the engine and the caller's gateway; neither outlives the call. Keeping the counters
/// and proposals here is what lets `resolve` read as three steps instead of one long loop.
struct ResolutionRun<'e, 'g, 'a> {
    engine: &'e ResolutionEngine,
    gateway: &'g Gateway<'a>,
    evidence: &'e Evidence,
    vetoes: HashSet<(i32, String, String)>,
    report: ResolutionReport,
    provider_reports: Vec<ProviderReport>,
    proposed: BTreeMap<i32, Vec<(usize, Claim)>>,
    proposed_sets: HashMap<(usize, i32), (BTreeSet<String>, HashSet<(String, String)>)>,
    resolved: HashSet<i32>,
}

impl<'e, 'g, 'a> ResolutionRun<'e, 'g, 'a> {
    async fn new(
        engine: &'e ResolutionEngine,
        entities: &[model::knowledge_entity::Model],
        gateway: &'g Gateway<'a>,
    ) -> anyhow::Result<Self> {
        // Loaded once, not per claim: a veto asked during admission is a claim never written; one
        // that is only replayed is a claim asserted and then withdrawn.
        let vetoes = engine.graph.vetoed().await?;
        Ok(Self {
            engine,
            gateway,
            evidence: engine.graph.evidence(),
            vetoes,
            report: ResolutionReport {
                considered: entities.len(),
                offline: gateway.offline(),
                ..Default::default()
            },
            provider_reports: Vec::new(),
            proposed: BTreeMap::new(),
            proposed_sets: HashMap::new(),
            resolved: HashSet::new(),
        })
    }

    async fn collect(&mut self, entities: &[model::knowledge_entity::Model]) -> anyhow::Result<()> {
        let entity_ids: Vec<i32> = entities.iter().map(|entity| entity.id).collect();

        for (index, resolver) in self.engine.resolvers.iter().enumerate() {
            // A provider that told us to stop gets no further questions.
            if matches!(self.gateway.refusal(), Some(Refusal::RateLimited { .. })) {
                break;
            }

            // This resolver's own claims, not any authority's: an entity Wikidata answered remains a
            // candidate for IMDb. The batch is computed before any writes, so a claim written below
            // cannot hide the entity from the next resolver.
            let claimed = if self.engine.options.include_already_claimed {
                HashSet::new()
            } else {
                self.evidence
                    .claimed_by(resolver.provider(), &entity_ids)
                    .await?
            };
            let batch: Vec<model::knowledge_entity::Model> = entities
                .iter()
                .filter(|entity| !claimed.contains(&entity.id))
                .cloned()
                .collect();
            let batch_ids: HashSet<i32> = batch.iter().map(|entity| entity.id).collect();

            let before = self.gateway.stats();
            let mut per = ProviderReport {
                provider: resolver.provider().to_string(),
                batch: batch.len(),
                ..Default::default()
            };
            if !batch.is_empty() {
                let mut per_entity: HashMap<i32, usize> = HashMap::new();
                for (knowledge_entity_id, claim) in resolver.resolve(&batch, self.gateway).await {
                    if !batch_ids.contains(&knowledge_entity_id) {
                        per.dropped += 1;
                        tracing::warn!(
                            provider = resolver.provider(),
                            knowledge_entity_id,
                            "resolver returned a claim outside its batch"
                        );
                        continue;
                    }
                    let count = per_entity.entry(knowledge_entity_id).or_default();
                    if *count >= MAX_CLAIMS_PER_ENTITY {
                        per.dropped += 1;
                        continue;
                    }
                    *count += 1;
                    let (schemes, keep) = self
                        .proposed_sets
                        .entry((index, knowledge_entity_id))
                        .or_default();
                    schemes.insert(claim.scheme.clone());
                    keep.insert((claim.scheme.clone(), claim.external_id.clone()));
                    self.proposed
                        .entry(knowledge_entity_id)
                        .or_default()
                        .push((index, claim));
                }
            }
            let after = self.gateway.stats();
            per.traffic = crate::fetch::Stats {
                requested: after.requested.saturating_sub(before.requested),
                cache_hits: after.cache_hits.saturating_sub(before.cache_hits),
                fetched: after.fetched.saturating_sub(before.fetched),
                failed: after.failed.saturating_sub(before.failed),
            };
            self.provider_reports.push(per);
        }
        Ok(())
    }

    async fn apply(&mut self) -> anyhow::Result<()> {
        // Entity-first apply: every authority's claims for an entity land before its projection.
        let proposed = std::mem::take(&mut self.proposed);
        for (knowledge_entity_id, claims) in proposed {
            for (resolver_index, claim) in claims {
                let basis = claim.basis;
                let method = claim.method.clone();
                let provider = self.engine.resolvers[resolver_index].provider();
                let draft = DraftIdentity::from_claim(claim, provider);
                let key = draft.key();
                let identity_id = self.engine.graph.upsert_identity(&draft).await?;
                match self
                    .engine
                    .graph
                    .assert(
                        ClaimDraft::authority(
                            knowledge_entity_id,
                            &key,
                            identity_id,
                            basis,
                            provider,
                            method,
                        ),
                        &self.vetoes,
                    )
                    .await?
                {
                    Stored::Written => {
                        self.report.written += 1;
                        self.provider_reports[resolver_index].written += 1;
                        self.resolved.insert(knowledge_entity_id);
                    }
                    Stored::Vetoed => {
                        self.report.vetoed += 1;
                        self.provider_reports[resolver_index].vetoed += 1;
                    }
                    Stored::AlreadyClaimed => {}
                }
            }

            if self.engine.options.replace && self.gateway.refusal().is_none() {
                for (index, resolver) in self.engine.resolvers.iter().enumerate() {
                    let Some((schemes, keep)) =
                        self.proposed_sets.get(&(index, knowledge_entity_id))
                    else {
                        continue;
                    };
                    self.provider_reports[index].replaced += self
                        .evidence
                        .retract_unproposed(knowledge_entity_id, resolver.provider(), schemes, keep)
                        .await?;
                }
            }

            // A later `proximity auto-merge` picks a survivor by the projected `confidence`, so an
            // entity that just earned its claims is projected here — after all authorities spoke.
            self.engine
                .graph
                .update_projection(knowledge_entity_id)
                .await?;
        }
        Ok(())
    }

    fn finish(mut self) -> ResolutionReport {
        // Resolving stops at evidence. Which holders of an identity are one thing belongs to
        // `proximity scan` and `proximity auto-merge`, which are separate commands run after this one.
        self.report.entities_resolved = self.resolved.len();
        self.report.by_provider = std::mem::take(&mut self.provider_reports);
        self.report.traffic = self.gateway.stats();
        self.report.refusal = self.gateway.refusal();
        self.report
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

    use crate::fetch::{
        Allowance, Answer, BATCH, FetchCache, FetchError, Outcome, Request, Transport,
    };
    use crate::knowledge::evidence::Claim;
    use crate::model;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::database;
    use crate::test_support::db_or_skip;

    /// Test-only alias for the composition the CLI uses. Production has no `run`; the helper is the
    /// operation, and the primitive is `resolve`.
    #[cfg(test)]
    impl ResolutionEngine {
        /// Production passes the whole list to `new`; tests build it one fake at a time.
        fn with(mut self, resolver: Box<dyn Resolver>) -> Self {
            self.resolvers.push(resolver);
            self
        }

        async fn resolve_pending(self) -> anyhow::Result<ResolutionReport> {
            self.resolve_pending_with(true).await
        }

        async fn resolve_pending_offline(self) -> anyhow::Result<ResolutionReport> {
            self.resolve_pending_with(false).await
        }

        async fn resolve_pending_with(self, online: bool) -> anyhow::Result<ResolutionReport> {
            let db = self.graph.db();
            let cache = crate::fetch::FetchCache::new(&db.conn);
            let allowance = online.then(|| {
                crate::fetch::Allowance::new(DEFAULT_MAX_REQUESTS, std::time::Duration::ZERO)
            });
            crate::knowledge::helper::resolve_all_pending(
                db,
                &self,
                &cache,
                allowance.as_ref(),
                None,
                false,
            )
            .await
        }
    }

    #[cfg(test)]
    async fn resolve_entities(
        engine: ResolutionEngine,
        entities: &[model::knowledge_entity::Model],
    ) -> anyhow::Result<ResolutionReport> {
        let db = engine.graph.db();
        let cache = crate::fetch::FetchCache::new(&db.conn);
        let allowance =
            crate::fetch::Allowance::new(DEFAULT_MAX_REQUESTS, std::time::Duration::ZERO);
        engine.resolve(entities, &cache, Some(&allowance)).await
    }

    /// Which candidates a finite budget reaches is a policy, so pin both orders.
    #[tokio::test]
    async fn candidates_are_ordered_by_the_run_not_by_habit() {
        db_or_skip!(db);
        let first = crate::test_support::fixtures::knowledge_entity_with_member(&db, "First").await;
        let second =
            crate::test_support::fixtures::knowledge_entity_with_member(&db, "Second").await;

        let evidence = crate::knowledge::evidence::Evidence::from_handle(&db);
        let ascending = evidence
            .awaiting_authority(&["wikidata"], Some(2), false)
            .await
            .unwrap();
        let descending = evidence
            .awaiting_authority(&["wikidata"], Some(2), true)
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
        fn provider(&self) -> &'static str {
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
                            external_id: id,
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

    /// A second authority with no transport: it always names the same IMDb id.
    struct Other;

    #[async_trait::async_trait]
    impl Resolver for Other {
        fn provider(&self) -> &'static str {
            "imdb"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            entities
                .iter()
                .map(|entity| {
                    (
                        entity.id,
                        Claim {
                            scheme: "imdb".to_string(),
                            external_id: "tt1".to_string(),
                            label: None,
                            description: None,
                            classes: Vec::new(),
                            method: "title".to_string(),
                            confidence: 0.8,
                            basis: crate::knowledge::evidence::ClaimBasis::Text,
                        },
                    )
                })
                .collect()
        }
    }

    /// An over-eager authority: ten identities for one entity.
    struct Many;

    #[async_trait::async_trait]
    impl Resolver for Many {
        fn provider(&self) -> &'static str {
            "many"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            entities
                .iter()
                .flat_map(|entity| {
                    (0..10).map(|index| {
                        (
                            entity.id,
                            Claim {
                                scheme: "imdb".to_string(),
                                external_id: format!("tt{index}"),
                                label: None,
                                description: None,
                                classes: Vec::new(),
                                method: "search".to_string(),
                                confidence: 0.8,
                                basis: crate::knowledge::evidence::ClaimBasis::Text,
                            },
                        )
                    })
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
        let answers = HashMap::from([("Train Dreams".to_string(), identified("Q185971"))]);

        let first = ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
            .expect("engine")
            .with(Box::new(Fake {
                answers: answers.clone(),
                calls: calls.clone(),
            }))
            .resolve_pending()
            .await
            .expect("resolve");
        assert_eq!(first.written, 1, "{first:?}");
        assert_eq!(first.traffic.fetched, 1, "{first:?}");
        assert_eq!(first.by_provider.len(), 1, "{first:?}");
        assert_eq!(first.by_provider[0].provider, "wikidata");
        assert_eq!(first.by_provider[0].written, 1);
        assert_eq!(first.by_provider[0].traffic.fetched, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let entity = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entity.status,
            crate::model::knowledge_entity::Status::Identified
        );
        assert_eq!(entity.confidence, Some(0.9));

        // Second run: the entity now has an ontology claim, so it is not even a candidate.
        let second =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Fake {
                    answers,
                    calls: calls.clone(),
                }))
                .resolve_pending()
                .await
                .expect("resolve");
        assert_eq!(second.considered, 0, "{second:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second fetch");
    }

    /// A second authority sees an entity the first one answered, and both claims survive.
    #[tokio::test]
    async fn two_authorities_may_assert_one_entity() {
        db_or_skip!(db);
        let entity = seed(&db, "Train Dreams").await;
        let answers = HashMap::from([("Train Dreams".to_string(), identified("Q1"))]);

        let engine =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Fake {
                    answers,
                    calls: Arc::new(AtomicUsize::new(0)),
                }))
                .with(Box::new(Other));
        let report = engine.resolve_pending().await.expect("resolve");

        assert_eq!(report.written, 2, "{report:?}");
        assert_eq!(report.by_provider.len(), 2, "{report:?}");
        assert_eq!(report.by_provider[0].provider, "wikidata");
        assert_eq!(report.by_provider[1].provider, "imdb");
        assert_eq!(
            report.by_provider[1].batch, 1,
            "the second authority is asked"
        );

        let providers: Vec<String> = model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity))
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|claim| claim.provider)
            .collect();
        assert!(providers.contains(&"wikidata".to_string()), "{providers:?}");
        assert!(providers.contains(&"imdb".to_string()), "{providers:?}");
    }

    /// An authority cannot inflate its evidence by returning a hundred identities for one entity.
    #[tokio::test]
    async fn a_resolver_may_not_exceed_the_per_entity_cap() {
        db_or_skip!(db);
        let id = seed(&db, "Capped").await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        let engine =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Many));
        let report = resolve_entities(engine, std::slice::from_ref(&entity))
            .await
            .expect("resolve");

        assert_eq!(report.written, MAX_CLAIMS_PER_ENTITY, "{report:?}");
        assert_eq!(report.by_provider[0].dropped, 2, "{report:?}");
        assert_eq!(
            model::knowledge_claims::Entity::find()
                .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
                .count(&db.conn)
                .await
                .unwrap(),
            MAX_CLAIMS_PER_ENTITY as u64
        );
    }

    /// A re-resolution that includes already-claimed entities may replace what the same authority
    /// said before, but only in the scheme it answered.
    #[tokio::test]
    async fn replace_retracts_the_old_claim_it_did_not_repropose() {
        db_or_skip!(db);
        let id = seed(&db, "Train Dreams").await;
        crate::test_support::fixtures::claim(
            &db,
            id,
            "imdb",
            "tt999",
            "imdb",
            Some("title"),
            Some(0.7),
            crate::model::knowledge_claims::ClaimBasis::Text,
        )
        .await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        let engine = ResolutionEngine::new(
            KnowledgeGraph::new(&db),
            Options {
                include_already_claimed: true,
                replace: true,
                ..Default::default()
            },
            Vec::new(),
        )
        .expect("engine")
        .with(Box::new(Other));
        let report = resolve_entities(engine, std::slice::from_ref(&entity))
            .await
            .expect("resolve");

        assert_eq!(report.written, 1, "{report:?}");
        assert_eq!(report.by_provider[0].replaced, 1, "{report:?}");

        let rows = model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .filter(|claim| claim.retracted_at.is_some())
                .count(),
            1,
            "the old claim is retracted"
        );
    }

    #[tokio::test]
    async fn an_empty_answer_is_cached_and_not_re_asked() {
        db_or_skip!(db);
        let knowledge_entity_id = seed(&db, "MUME").await;

        let calls = Arc::new(AtomicUsize::new(0));

        let first = ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
            .expect("engine")
            .with(Box::new(Fake {
                answers: HashMap::new(),
                calls: calls.clone(),
            }))
            .resolve_pending()
            .await
            .expect("resolve");
        assert_eq!(first.traffic.fetched, 1, "{first:?}");
        assert_eq!(first.written, 0);

        // The entity is still a candidate (no claim), but the negative answer is cached.
        let second =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Fake {
                    answers: HashMap::new(),
                    calls: calls.clone(),
                }))
                .resolve_pending()
                .await
                .expect("resolve");
        assert_eq!(second.considered, 1, "{second:?}");
        assert_eq!(second.traffic.fetched, 0, "{second:?}");
        assert_eq!(second.traffic.cache_hits, 1, "{second:?}");
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
        assert_eq!(
            entity.status,
            crate::model::knowledge_entity::Status::Provisional
        );
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
        let report =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Fake {
                    answers: HashMap::new(),
                    calls: calls.clone(),
                }))
                .resolve_pending_offline()
                .await
                .expect("resolve");

        assert!(report.offline, "the run says it was cache-only");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "offline never touches the transport"
        );
        assert_eq!(report.traffic.fetched, 0);
        assert_eq!(report.traffic.cache_hits, 1);
        assert_eq!(
            report.written, 1,
            "only the cached one resolves: {report:?}"
        );

        let entity = model::knowledge_entity::Entity::find_by_id(cached)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entity.status,
            crate::model::knowledge_entity::Status::Identified,
            "{entity:?}"
        );

        // The miss is left alone, which is the whole point: offline builds what the cache can and
        // stops there.
        let entity = model::knowledge_entity::Entity::find_by_id(uncached)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            entity.status,
            crate::model::knowledge_entity::Status::Provisional,
            "{entity:?}"
        );
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
            crate::model::knowledge_claims::ClaimBasis::Text,
        )
        .await;
        crate::test_support::fixtures::claim(
            &db,
            knowledge_entity_id,
            "x_twitter",
            "nintendo",
            crate::knowledge::evidence::ILLUMINATOR,
            None,
            None,
            crate::model::knowledge_claims::ClaimBasis::Handle,
        )
        .await;

        crate::knowledge::KnowledgeGraph::new(&db)
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
            (
                crate::knowledge::evidence::ILLUMINATOR,
                None,
                "x_twitter",
                "tsoding",
                None,
            ),
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
                crate::knowledge::evidence::ClaimBasis::Handle,
            )
            .await;
        }

        crate::knowledge::KnowledgeGraph::new(&db)
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
        fn provider(&self) -> &'static str {
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
        fn provider(&self) -> &'static str {
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
                                external_id: format!("Q{}", entity.name.len()),
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

        let report =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Flaky {
                    calls: Arc::new(AtomicUsize::new(0)),
                }))
                .resolve_pending()
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
            report.traffic.fetched, 2,
            "one successful batch, then the 429: {report:?}"
        );
        assert_eq!(
            report.written, BATCH,
            "claims from the first batch survive: {report:?}"
        );
    }

    #[tokio::test]
    async fn a_rate_limit_stops_the_run_and_is_not_cached() {
        db_or_skip!(db);
        seed(&db, "Train Dreams").await;

        let report =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Limited))
                .resolve_pending()
                .await
                .expect("resolve");

        assert_eq!(
            report.refusal,
            Some(Refusal::RateLimited {
                retry_after_secs: Some(30)
            }),
            "{report:?}"
        );
        assert_eq!(report.traffic.fetched, 1, "{report:?}");
        assert_eq!(report.written, 0, "{report:?}");
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
            crate::model::knowledge_claims::ClaimBasis::Text,
        )
        .await;
        crate::test_support::fixtures::claim(
            &db,
            earns_now,
            "x_twitter",
            "traindreams",
            crate::knowledge::evidence::ILLUMINATOR,
            None,
            None,
            crate::model::knowledge_claims::ClaimBasis::Handle,
        )
        .await;

        // The state a resolve starts from: both projected, so the claimless one reads as 0.0.
        let store = crate::knowledge::KnowledgeGraph::new(&db);
        store.update_projection(resolved_before).await.unwrap();
        store.update_projection(earns_now).await.unwrap();

        let claim = identified("Q42");
        ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
            .expect("engine")
            .with(Box::new(Fake {
                answers: HashMap::from([("Train Dreams".to_string(), claim)]),
                calls: Arc::new(AtomicUsize::new(0)),
            }))
            .resolve_pending()
            .await
            .expect("resolve");

        // The passes are separate commands now, so the ordering guarantee is exercised the way the
        // CLI runs it: resolve projects, then scan raises the pair, then merge folds it.
        let scoring = crate::knowledge::linker::heuristic::Heuristic::scoring();
        crate::knowledge::proximity::scan(&db.conn, &scoring)
            .await
            .expect("scan");
        let folded = crate::knowledge::proximity::auto_merge(&db.conn, &store, &scoring, true)
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

        let report =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Fake {
                    answers,
                    calls: Arc::new(AtomicUsize::new(0)),
                }))
                .resolve_pending()
                .await
                .expect("resolve");

        assert_eq!(report.written, 2, "{report:?}");

        // The proximity passes now run after resolve as separate commands; drive them explicitly so
        // the assertion is about `merge` refusing a pair, not about an empty batch.
        let store = crate::knowledge::KnowledgeGraph::new(&db);
        let scoring = crate::knowledge::linker::heuristic::Heuristic::scoring();
        crate::knowledge::proximity::scan(&db.conn, &scoring)
            .await
            .expect("scan");
        let folded = crate::knowledge::proximity::auto_merge(&db.conn, &store, &scoring, true)
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

    /// A resolver id is a provider id. `illuminator` names our own pipeline, so taking it would
    /// make capture rows look like this resolver's claims.
    struct Reserved;

    #[async_trait::async_trait]
    impl Resolver for Reserved {
        fn provider(&self) -> &'static str {
            "illuminator"
        }

        async fn resolve(
            &self,
            _entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn a_resolver_cannot_take_a_reserved_provider() {
        db_or_skip!(db);
        let error = ResolutionEngine::new(
            KnowledgeGraph::new(&db),
            Options::default(),
            vec![Box::new(Reserved)],
        )
        .err()
        .expect("the reserved id must be refused");
        assert!(error.to_string().contains("reserved"), "{error}");
    }
}
