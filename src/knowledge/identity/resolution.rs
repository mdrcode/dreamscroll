//! Resolution: the authority contract, and the run loop that drives it.
//!
//! A resolver owns its strategy — one request or a ladder of them, one pass or several. `Gateway` is
//! its only door to the world: the gateway consults the cache first, paces the call and charges the
//! budget. The engine selects the entities, gives each resolver a batch, and stores what comes back.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::fetch::{self, Allowance, FetchCache, Gateway, Refusal};
use crate::knowledge::KnowledgeGraph;
use crate::knowledge::evidence::{Claim, ClaimOutcome, Evidence, IdentityDraft};
use crate::model;

use super::{MAX_CLAIMS_PER_ENTITY, RESERVED_PROVIDER_IDS};

/// One authority and its strategy for identifying entities.
///
/// `resolve` takes a *batch* of entities rather than one: batching across entities is the difference
/// between a hundred transport calls for a corpus walk and five thousand, and only the resolver knows
/// how to group its own requests.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    /// The authority's id: the `provider` on every request and every identity it writes. Reserved
    /// ids are refused by [`ResolutionEngine::new`].
    fn id(&self) -> &'static str;

    /// Discover identities for the entities, returning each identity and the claim to it.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(IdentityDraft, Claim)>;
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Ask a resolver again even when it already has a live claim for the entity. `--entity` sets
    /// this; without it, the per-provider claimed filter makes an entity-scoped run empty.
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
    pub offline: bool,

    pub considered: usize,
    pub fetch_traffic: fetch::Stats,

    pub entities_resolved: usize,
    pub claims_written: usize,
    pub claims_vetoed: usize,

    pub by_provider: Vec<ProviderReport>,
    /// The run-wide budget refusal, or a provider's rate limit, for the report.
    pub refusal: Option<Refusal>,
}

/// What one provider did in one run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProviderReport {
    pub provider: String,
    pub batch: usize,
    pub fetch_traffic: fetch::Stats,

    pub claims_written: usize,
    pub claims_vetoed: usize,

    /// Claims refused by the per-entity cap.
    pub dropped: usize,
    /// Old claims this provider owned that the re-resolve did not re-propose.
    pub replaced: usize,
}

/// The harness: it holds the resolvers, invokes them, and stores their output.
pub struct ResolutionEngine {
    graph: KnowledgeGraph,
    options: Options,
    resolvers: Vec<Box<dyn Resolver>>,
}

impl ResolutionEngine {
    pub fn new(
        graph: KnowledgeGraph,
        options: Options,
        resolvers: Vec<Box<dyn Resolver>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !options.replace || options.include_already_claimed,
            "replace requires include_already_claimed"
        );
        let mut ids = HashSet::new();
        for resolver in &resolvers {
            anyhow::ensure!(
                !RESERVED_PROVIDER_IDS.contains(&resolver.id()),
                "resolver id `{}` is reserved; it names our own pipeline, not an authority",
                resolver.id()
            );
            anyhow::ensure!(
                ids.insert(resolver.id()),
                "resolver id `{}` is used twice; provider-scoped state assumes one resolver each",
                resolver.id()
            );
        }
        Ok(Self {
            graph,
            options,
            resolvers,
        })
    }

    pub(crate) fn providers(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.resolvers.iter().map(|resolver| resolver.id())
    }

    /// Resolve exactly the entities in scope; selection is the caller's job.
    pub async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        cache: &FetchCache<'_>,
        allowance: Option<&Allowance>,
    ) -> anyhow::Result<ResolutionReport> {
        // The caller may hand in a tombstone or a duplicate; neither is a question to ask.
        let mut seen = HashSet::new();
        let entities: Vec<model::knowledge_entity::Model> = entities
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

/// The mutable state of one engine-level `resolve`.
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
    proposed: BTreeMap<i32, Vec<(usize, IdentityDraft, Claim)>>,
    resolved: HashSet<i32>,
    capped: HashSet<(usize, i32)>,
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
            resolved: HashSet::new(),
            capped: HashSet::new(),
        })
    }

    async fn collect(&mut self, entities: &[model::knowledge_entity::Model]) -> anyhow::Result<()> {
        let entity_ids: Vec<i32> = entities.iter().map(|entity| entity.id).collect();

        for (index, resolver) in self.engine.resolvers.iter().enumerate() {
            match self.gateway.refusal_for(resolver.id()) {
                Some(Refusal::Budget) => break,
                Some(Refusal::RateLimited { .. }) => continue,
                None => {}
            }

            let already_claimed = if self.engine.options.include_already_claimed {
                HashSet::new()
            } else {
                self.evidence
                    .selection()
                    .claimed_by(resolver.id(), &entity_ids)
                    .await?
            };
            let batch: Vec<model::knowledge_entity::Model> = entities
                .iter()
                .filter(|entity| !already_claimed.contains(&entity.id))
                .cloned()
                .collect();
            let batch_ids: HashSet<i32> = batch.iter().map(|entity| entity.id).collect();

            let before = self.gateway.stats();
            let mut report = ProviderReport {
                provider: resolver.id().to_string(),
                batch: batch.len(),
                ..Default::default()
            };
            if !batch.is_empty() {
                let mut per_entity: HashMap<i32, usize> = HashMap::new();

                for (identity_draft, claim) in resolver.resolve(&batch, self.gateway).await {
                    let knowledge_entity_id = claim.holder;
                    if !batch_ids.contains(&knowledge_entity_id) {
                        tracing::warn!(
                            provider = resolver.id(),
                            knowledge_entity_id,
                            "resolver returned a claim outside its batch"
                        );
                        continue;
                    }
                    anyhow::ensure!(
                        claim.provider == resolver.id(),
                        "resolver `{}` returned a claim attributed to `{}`",
                        resolver.id(),
                        claim.provider
                    );
                    let count = per_entity.entry(knowledge_entity_id).or_default();
                    if *count >= MAX_CLAIMS_PER_ENTITY {
                        report.dropped += 1;
                        if self.engine.options.replace {
                            self.capped.insert((index, knowledge_entity_id));
                        }
                        continue;
                    }
                    *count += 1;
                    self.proposed.entry(knowledge_entity_id).or_default().push((
                        index,
                        identity_draft,
                        claim,
                    ));
                }
            }
            let after = self.gateway.stats();
            report.fetch_traffic = fetch::Stats {
                requested: after.requested.saturating_sub(before.requested),
                cache_hits: after.cache_hits.saturating_sub(before.cache_hits),
                fetched: after.fetched.saturating_sub(before.fetched),
                failed: after.failed.saturating_sub(before.failed),
            };
            self.provider_reports.push(report);
        }
        Ok(())
    }

    async fn apply(&mut self) -> anyhow::Result<()> {
        // Entity-first apply: every authority's claims for an entity land before its projection.
        // A shared identity's confidence is an input to every holder's projection. The entity that
        // wrote the answer is projected below; these are the identities whose holders need it.
        let mut touched_identities: HashSet<i32> = HashSet::new();
        let proposed = std::mem::take(&mut self.proposed);
        for (knowledge_entity_id, claims) in proposed {
            let mut keep_by_resolver: HashMap<usize, HashSet<(String, String)>> = HashMap::new();
            for (resolver_index, identity_draft, claim) in claims {
                let key = identity_draft.key();
                let claimed = self
                    .engine
                    .graph
                    .claim_identity(identity_draft, claim, &self.vetoes)
                    .await?;
                let identity_id = match claimed {
                    ClaimOutcome::Written { identity_id, .. }
                    | ClaimOutcome::Vetoed { identity_id }
                    | ClaimOutcome::AlreadyClaimed { identity_id, .. } => identity_id,
                };
                touched_identities.insert(identity_id);
                match claimed {
                    ClaimOutcome::Written { .. } => {
                        self.report.claims_written += 1;
                        self.provider_reports[resolver_index].claims_written += 1;
                        self.resolved.insert(knowledge_entity_id);
                    }
                    ClaimOutcome::Vetoed { .. } => {
                        self.report.claims_vetoed += 1;
                        self.provider_reports[resolver_index].claims_vetoed += 1;
                    }
                    ClaimOutcome::AlreadyClaimed { .. } => {}
                }
                if self.engine.options.replace && !matches!(claimed, ClaimOutcome::Vetoed { .. }) {
                    keep_by_resolver
                        .entry(resolver_index)
                        .or_default()
                        .insert((key.scheme, key.external_id));
                }
            }

            if self.engine.options.replace {
                for (index, keep) in keep_by_resolver {
                    if self.capped.contains(&(index, knowledge_entity_id)) {
                        continue;
                    }
                    let provider = self.engine.resolvers[index].id();
                    if self.gateway.refusal_for(provider).is_some() {
                        continue;
                    }
                    self.provider_reports[index].replaced += self
                        .evidence
                        .retract_unproposed(knowledge_entity_id, provider, &keep)
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
        for identity_id in touched_identities {
            self.engine
                .graph
                .reproject_identity_holders(identity_id)
                .await?;
        }
        Ok(())
    }

    fn finish(mut self) -> ResolutionReport {
        // Resolving stops at evidence. Which holders of an identity are one thing belongs to
        // `proximity scan` and `proximity auto-merge`, which are separate commands run after this one.
        self.report.entities_resolved = self.resolved.len();
        self.report.by_provider = std::mem::take(&mut self.provider_reports);
        self.report.fetch_traffic = self.gateway.stats();
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
    use crate::knowledge::evidence::IdentityKey;
    use crate::knowledge::identity::DEFAULT_MAX_REQUESTS;
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

        let selection = crate::knowledge::evidence::ClaimSelection::from_handle(&db);
        let ascending = selection
            .awaiting_authority(&["wikidata"], Some(2), false)
            .await
            .unwrap();
        let descending = selection
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

    #[allow(clippy::too_many_arguments)]
    fn resolved(
        holder: i32,
        provider: &'static str,
        scheme: &str,
        external_id: &str,
        label: Option<String>,
        method: &str,
        confidence: f32,
    ) -> (IdentityDraft, Claim) {
        let key = IdentityKey {
            scheme: scheme.to_string(),
            external_id: external_id.to_string(),
        };
        let identity_draft = IdentityDraft {
            scheme: scheme.to_string(),
            external_id: external_id.to_string(),
            provider: provider.to_string(),
            method: Some(method.to_string()),
            confidence: Some(confidence),
            label,
            description: None,
            classes: Vec::new(),
        };
        (
            identity_draft,
            Claim::from_provider(holder, key, provider, method.to_string()),
        )
    }

    struct Fake {
        answers: HashMap<String, Answer<Vec<String>>>,
        calls: Arc<AtomicUsize>,
    }

    fn fake_request(
        provider: &'static str,
        entity: &model::knowledge_entity::Model,
    ) -> Request<Probe> {
        Request {
            provider,
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
        ) -> Vec<(IdentityDraft, Claim)> {
            let requests: Vec<Request<Probe>> = entities
                .iter()
                .map(|entity| fake_request(self.id(), entity))
                .collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            entities
                .iter()
                .filter_map(|entity| {
                    let raw = answers.get(&fake_request(self.id(), entity))?;
                    let id = raw.body.as_ref()?.first()?.clone();
                    Some(resolved(
                        entity.id,
                        self.id(),
                        "wikidata",
                        &id,
                        Some(entity.name.clone()),
                        "exact_label",
                        0.9,
                    ))
                })
                .collect()
        }
    }

    /// A second authority with no transport: it always names the same IMDb id.
    struct Other;

    #[async_trait::async_trait]
    impl Resolver for Other {
        fn id(&self) -> &'static str {
            "imdb"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(IdentityDraft, Claim)> {
            entities
                .iter()
                .map(|entity| resolved(entity.id, self.id(), "imdb", "tt1", None, "title", 0.8))
                .collect()
        }
    }

    /// An over-eager authority: ten identities for one entity.
    struct Many;

    #[async_trait::async_trait]
    impl Resolver for Many {
        fn id(&self) -> &'static str {
            "many"
        }

        async fn resolve(
            &self,
            entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(IdentityDraft, Claim)> {
            entities
                .iter()
                .flat_map(|entity| {
                    (0..10).map(|index| {
                        resolved(
                            entity.id,
                            self.id(),
                            "imdb",
                            &format!("tt{index}"),
                            None,
                            "search",
                            0.8,
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
        assert_eq!(first.claims_written, 1, "{first:?}");
        assert_eq!(first.fetch_traffic.fetched, 1, "{first:?}");
        assert_eq!(first.by_provider.len(), 1, "{first:?}");
        assert_eq!(first.by_provider[0].provider, "wikidata");
        assert_eq!(first.by_provider[0].claims_written, 1);
        assert_eq!(first.by_provider[0].fetch_traffic.fetched, 1);
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

        assert_eq!(report.claims_written, 2, "{report:?}");
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

        assert_eq!(report.claims_written, MAX_CLAIMS_PER_ENTITY, "{report:?}");
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

        assert_eq!(report.claims_written, 1, "{report:?}");
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
    async fn a_vetoed_replacement_does_not_retract_the_old_claim() {
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
        )
        .await;
        KnowledgeGraph::new(&db)
            .veto(id, "imdb", "tt1", Some("reject the replacement"))
            .await
            .expect("record the veto");

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

        assert_eq!(report.claims_vetoed, 1, "{report:?}");
        assert_eq!(report.claims_written, 0, "{report:?}");
        assert_eq!(report.by_provider[0].replaced, 0, "{report:?}");

        let rows = model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "the vetoed proposal was not written, so the old claim has no replacement"
        );
        assert!(
            rows[0].retracted_at.is_none(),
            "a vetoed proposal is not evidence that the old claim is stale"
        );
    }

    #[tokio::test]
    async fn a_capped_answer_does_not_replace() {
        db_or_skip!(db);
        let id = seed(&db, "Train Dreams").await;
        crate::test_support::fixtures::claim(
            &db,
            id,
            "many",
            "tt9",
            "imdb",
            Some("title"),
            Some(0.7),
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
        .with(Box::new(Many));
        let report = resolve_entities(engine, std::slice::from_ref(&entity))
            .await
            .expect("resolve");

        assert_eq!(report.by_provider[0].dropped, 2, "{report:?}");
        assert_eq!(report.by_provider[0].replaced, 0, "{report:?}");

        let rows = model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            MAX_CLAIMS_PER_ENTITY + 1,
            "the capped claims are not written, and the old claim is not retracted"
        );
        assert!(
            rows.iter().all(|claim| claim.retracted_at.is_none()),
            "a truncated answer cannot justify a retraction"
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
        assert_eq!(first.fetch_traffic.fetched, 1, "{first:?}");
        assert_eq!(first.claims_written, 0);

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
        assert_eq!(second.fetch_traffic.fetched, 0, "{second:?}");
        assert_eq!(second.fetch_traffic.cache_hits, 1, "{second:?}");
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
                "wikidata",
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
        assert_eq!(report.fetch_traffic.fetched, 0);
        assert_eq!(report.fetch_traffic.cache_hits, 1);
        assert_eq!(
            report.claims_written, 1,
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

    /// Confidence is the best claim; a second claim does not add a bonus.
    #[tokio::test]
    async fn confidence_is_the_best_claim() {
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
        )
        .await;
        crate::test_support::fixtures::claim(
            &db,
            knowledge_entity_id,
            "x_twitter",
            "nintendo",
            crate::knowledge::evidence::PLATFORM_LINK_PROVIDER_ID,
            None,
            None,
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
            (entity.confidence.unwrap() - 0.8).abs() < 1e-6,
            "{entity:?}"
        );
    }

    /// A shared identity's confidence is an input to every holder's projection. Resolving one holder
    /// must reproject the others, or `auto_merge` reads a stale survivor.
    #[tokio::test]
    async fn a_shared_identity_change_reprojects_every_holder() {
        db_or_skip!(db);
        let first =
            crate::test_support::fixtures::knowledge_entity_with_member(&db, "First Holder").await;
        let second =
            crate::test_support::fixtures::knowledge_entity_with_member(&db, "Second Holder").await;

        for entity in [first, second] {
            crate::test_support::fixtures::claim(
                &db,
                entity,
                "wikidata",
                "Q42",
                "wikidata",
                Some("exact_label"),
                Some(0.5),
            )
            .await;
        }

        let store = KnowledgeGraph::new(&db);
        store.update_projection(first).await.unwrap();
        store.update_projection(second).await.unwrap();
        assert!((store.get(first).await.unwrap().confidence.unwrap() - 0.5).abs() < 1e-6);
        assert!((store.get(second).await.unwrap().confidence.unwrap() - 0.5).abs() < 1e-6);

        let entity = model::knowledge_entity::Entity::find_by_id(second)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let engine = ResolutionEngine::new(
            KnowledgeGraph::new(&db),
            Options {
                include_already_claimed: true,
                ..Default::default()
            },
            Vec::new(),
        )
        .expect("engine")
        .with(Box::new(Fake {
            answers: HashMap::from([("Second Holder".to_string(), identified("Q42"))]),
            calls: Arc::new(AtomicUsize::new(0)),
        }));
        resolve_entities(engine, &[entity]).await.expect("resolve");

        for entity_id in [first, second] {
            let entity = store.get(entity_id).await.unwrap();
            assert!(
                (entity.confidence.unwrap() - 0.9).abs() < 1e-6,
                "every holder takes the new identity confidence: {entity:?}"
            );
        }
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
        ) -> Vec<(IdentityDraft, Claim)> {
            let requests: Vec<Request<Probe>> = entities
                .iter()
                .map(|entity| fake_request(self.id(), entity))
                .collect();
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
        ) -> Vec<(IdentityDraft, Claim)> {
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
                        resolved(
                            entity.id,
                            self.id(),
                            "wikidata",
                            &format!("Q{}", entity.name.len()),
                            Some(entity.name.clone()),
                            "exact_label",
                            0.9,
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
            report.fetch_traffic.fetched, 2,
            "one successful batch, then the 429: {report:?}"
        );
        assert_eq!(
            report.claims_written, BATCH,
            "claims from the first batch survive: {report:?}"
        );
    }

    #[tokio::test]
    async fn a_rate_limit_stops_that_provider_and_is_not_cached() {
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
        assert_eq!(report.fetch_traffic.fetched, 1, "{report:?}");
        assert_eq!(report.claims_written, 0, "{report:?}");
        assert_eq!(
            model::fetch_cache::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            0,
            "a rate limit must not be recorded as an answer"
        );
    }

    #[tokio::test]
    async fn a_rate_limit_on_one_provider_does_not_stop_the_next() {
        db_or_skip!(db);
        seed(&db, "Train Dreams").await;

        let report =
            ResolutionEngine::new(KnowledgeGraph::new(&db), Options::default(), Vec::new())
                .expect("engine")
                .with(Box::new(Limited))
                .with(Box::new(Other))
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
        assert_eq!(report.by_provider.len(), 2, "{report:?}");
        assert_eq!(report.by_provider[0].provider, "limited");
        assert_eq!(report.by_provider[0].claims_written, 0);
        assert_eq!(report.by_provider[1].provider, "imdb");
        assert_eq!(report.by_provider[1].claims_written, 1);
        assert_eq!(report.claims_written, 1, "the second provider still ran");
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

        assert_eq!(report.claims_written, 2, "{report:?}");

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
        fn id(&self) -> &'static str {
            "illuminator"
        }

        async fn resolve(
            &self,
            _entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(IdentityDraft, Claim)> {
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

    #[tokio::test]
    async fn a_resolver_id_must_be_unique() {
        db_or_skip!(db);
        let error = ResolutionEngine::new(
            KnowledgeGraph::new(&db),
            Options::default(),
            vec![Box::new(Other), Box::new(Other)],
        )
        .err()
        .expect("a repeated id must be refused");
        assert!(error.to_string().contains("used twice"), "{error}");
    }
}
