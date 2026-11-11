//! Resolution: the authority contract, and the run loop that drives it.
//!
//! A resolver owns its strategy — one request or a ladder of them, one pass or several. `Gateway` is
//! its only door to the world: the gateway consults the cache first, paces the call and charges the
//! budget. The engine selects the entities, gives each resolver a batch, and stores what comes back.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::RESERVED_PROVIDERS;
use crate::database;
use crate::fetch::{Allowance, FetchCache, Gateway, Refusal};
use crate::knowledge::evidence::{Claim, ClaimDraft, DraftIdentity, Evidence, Stored};
use crate::model;

/// Transport calls a plain resolve is allowed to make. Exists so that omitting the budget does real
/// work instead of quietly resolving nothing.
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
    /// [`RESERVED_PROVIDERS`]; the engine refuses those at run time.
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
    /// How many pending entities `resolve_all_pending` considers.
    pub limit: Option<usize>,
    /// Transport calls allowed in this run — the anti-spam budget.
    pub max_requests: usize,
    /// Minimum wall-clock gap between transport calls. One request per second keeps a walk of a few
    /// thousand labels to a few minutes while staying far below WDQS's processing-time budget.
    pub min_interval: std::time::Duration,
    pub offline: bool,
    /// Ask every resolver even when it already claimed the entity. `resolve --entity` sets this;
    /// already-held identities are no-ops, new ones are added.
    pub force: bool,
    /// Retract this provider's old claims in the schemes it answered but did not re-propose.
    /// Requires `force`; `resolve --entity --replace` sets both.
    pub replace: bool,
    /// Consider the newest pending entities first.
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
            force: false,
            replace: false,
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
    /// One entry per resolver called, so a failure or a silent provider is attributable.
    pub by_resolver: Vec<ResolverReport>,
    /// Why the run stopped early, if it did: the budget, or the provider's `429`. The caller prints
    /// what it needs from it, instead of this report carrying three derived answers to one question.
    pub refusal: Option<Refusal>,
    pub offline: bool,
}

/// What one resolver did in one run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResolverReport {
    pub provider: String,
    pub candidates: usize,
    pub claims: usize,
    pub vetoed: usize,
    /// Claims refused by the per-entity cap or returned outside the batch.
    pub dropped: usize,
    /// Old claims this provider owned that the forced run did not re-propose.
    pub replaced: usize,
    /// Deltas around this resolver's `resolve` call; the global counters remain on [`Report`].
    pub requested: usize,
    pub cache_hits: usize,
    pub fetched: usize,
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

    pub(crate) fn db(&self) -> &database::DbHandle {
        self.db
    }

    pub(crate) fn options(&self) -> &Options {
        &self.options
    }

    pub(crate) fn providers(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.resolvers.iter().map(|resolver| resolver.provider())
    }

    /// Resolve exactly the entities in scope; selection is the caller's job.
    ///
    /// The helper layer composes the scopes: one illumination, or the pending world. This method
    /// owns the run: reserved-id checks, the gateway, the vetoes, and the entity-first apply.
    pub async fn resolve(
        &self,
        entities: &[crate::model::knowledge_entity::Model],
    ) -> anyhow::Result<Report> {
        // A resolver may not take an id that names our own pipeline: selection and stats compare
        // this string to the identity's `provider`, so a collision would misattribute rows.
        anyhow::ensure!(
            !self.options.replace || self.options.force,
            "replace requires a forced resolve"
        );
        for resolver in &self.resolvers {
            anyhow::ensure!(
                !RESERVED_PROVIDERS.contains(&resolver.provider()),
                "resolver id `{}` is reserved; it names our own pipeline, not an authority",
                resolver.provider()
            );
        }

        // The caller may hand in a tombstone or a duplicate; neither is a question to ask.
        let mut seen = HashSet::new();
        let entities: Vec<crate::model::knowledge_entity::Model> = entities
            .iter()
            .filter(|entity| entity.merged_into_id.is_none() && seen.insert(entity.id))
            .cloned()
            .collect();

        let graph = crate::knowledge::KnowledgeGraph::new(self.db);
        // Loaded once, not per claim: a veto asked during admission is a claim never written; one
        // that is only replayed is a claim asserted and then withdrawn.
        let vetoes = graph.vetoed().await?;
        let cache = FetchCache::new(&self.db.conn);
        // The engine owns *which* entities to ask about. What the run may spend is the allowance's,
        // passed in; the gateway only asks it before each call. A cache-only run has none at all.
        let allowance = (!self.options.offline).then(|| {
            Allowance::new(self.options.max_requests, self.options.min_interval)
        });
        let gateway = Gateway::new(Some(&cache), allowance.as_ref(), true);
        let evidence = Evidence::from_handle(self.db);
        let mut report = Report {
            candidates: entities.len(),
            offline: self.options.offline,
            ..Default::default()
        };
        let entity_ids: Vec<i32> = entities.iter().map(|entity| entity.id).collect();
        let mut resolver_reports: Vec<ResolverReport> = Vec::new();
        let mut proposed: BTreeMap<i32, Vec<(usize, Claim)>> = BTreeMap::new();
        let mut proposed_sets: HashMap<
            (usize, i32),
            (BTreeSet<String>, HashSet<(String, String)>),
        > = HashMap::new();

        for (index, resolver) in self.resolvers.iter().enumerate() {
            // A provider that told us to stop gets no further questions.
            if matches!(
                allowance.as_ref().and_then(Allowance::refused),
                Some(Refusal::RateLimited { .. })
            ) {
                break;
            }

            // This resolver's own claims, not any authority's: an entity Wikidata answered remains a
            // candidate for IMDb. The batch is computed before any writes, so a claim written below
            // cannot hide the entity from the next resolver.
            let claimed = if self.options.force {
                HashSet::new()
            } else {
                evidence.claimed_by(resolver.provider(), &entity_ids).await?
            };
            let batch: Vec<model::knowledge_entity::Model> = entities
                .iter()
                .filter(|entity| !claimed.contains(&entity.id))
                .cloned()
                .collect();
            let batch_ids: HashSet<i32> = batch.iter().map(|entity| entity.id).collect();

            let before = gateway.stats();
            let mut per = ResolverReport {
                provider: resolver.provider().to_string(),
                candidates: batch.len(),
                ..Default::default()
            };
            if !batch.is_empty() {
                let mut per_entity: HashMap<i32, usize> = HashMap::new();
                for (knowledge_entity_id, claim) in resolver.resolve(&batch, &gateway).await {
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
                    let (schemes, keep) = proposed_sets
                        .entry((index, knowledge_entity_id))
                        .or_default();
                    schemes.insert(claim.scheme.clone());
                    keep.insert((claim.scheme.clone(), claim.external_id.clone()));
                    proposed
                        .entry(knowledge_entity_id)
                        .or_default()
                        .push((index, claim));
                }
            }
            let after = gateway.stats();
            per.requested = after.requested.saturating_sub(before.requested);
            per.cache_hits = after.cache_hits.saturating_sub(before.cache_hits);
            per.fetched = after.fetched.saturating_sub(before.fetched);
            resolver_reports.push(per);
        }

        // Entity-first apply: every authority's claims for an entity land before its projection.
        let mut resolved: HashSet<i32> = HashSet::new();
        for (knowledge_entity_id, claims) in proposed {
            for (resolver_index, claim) in claims {
                let basis = claim.basis;
                let method = claim.method.clone();
                let provider = self.resolvers[resolver_index].provider();
                let draft = DraftIdentity::from_claim(claim, provider);
                let key = draft.key();
                let identity_id = graph.upsert_identity(&draft).await?;
                match graph
                    .assert(
                        ClaimDraft::authority(
                            knowledge_entity_id,
                            &key,
                            identity_id,
                            basis,
                            provider,
                            method,
                        ),
                        &vetoes,
                    )
                    .await?
                {
                    Stored::Written => {
                        report.claims += 1;
                        resolver_reports[resolver_index].claims += 1;
                        resolved.insert(knowledge_entity_id);
                    }
                    Stored::Vetoed => {
                        report.vetoed += 1;
                        resolver_reports[resolver_index].vetoed += 1;
                    }
                    Stored::AlreadyClaimed => {}
                }
            }

            if self.options.replace
                && allowance.as_ref().and_then(Allowance::refused).is_none()
            {
                for (index, resolver) in self.resolvers.iter().enumerate() {
                    let Some((schemes, keep)) =
                        proposed_sets.get(&(index, knowledge_entity_id))
                    else {
                        continue;
                    };
                    resolver_reports[index].replaced += evidence
                        .retract_unproposed(
                            knowledge_entity_id,
                            resolver.provider(),
                            schemes,
                            keep,
                        )
                        .await?;
                }
            }

            // A later `proximity auto-merge` picks a survivor by the projected `confidence`, so an
            // entity that just earned its claims is projected here — after all authorities spoke.
            graph.update_projection(knowledge_entity_id).await?;
        }

        // Resolving stops at evidence. Which holders of an identity are one thing belongs to
        // `proximity scan` and `proximity auto-merge`, which are separate commands run after this one.
        report.entities_resolved = resolved.len();
        report.by_resolver = resolver_reports;
        report.stats = gateway.stats();
        report.refusal = allowance.as_ref().and_then(Allowance::refused);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

    use crate::fetch::{Answer, BATCH, FetchError, Outcome, Request, Transport};
    use crate::knowledge::evidence::Claim;
    use crate::model;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::test_support::db_or_skip;

    /// Test-only alias for the composition the CLI uses. Production has no `run`; the helper is the
    /// operation, and the primitive is `resolve`.
    #[cfg(test)]
    impl ResolutionEngine<'_> {
        async fn resolve_pending(self) -> anyhow::Result<Report> {
            crate::knowledge::helper::resolve_all_pending(&self).await
        }
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
        let answers = HashMap::from([(
            "Train Dreams".to_string(),
            identified("Q185971"),
        )]);

        let first = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers: answers.clone(),
                calls: calls.clone(),
            }))
            .resolve_pending()
            .await
            .expect("resolve");
        assert_eq!(first.claims, 1, "{first:?}");
        assert_eq!(first.stats.fetched, 1, "{first:?}");
        assert_eq!(first.by_resolver.len(), 1, "{first:?}");
        assert_eq!(first.by_resolver[0].provider, "wikidata");
        assert_eq!(first.by_resolver[0].claims, 1);
        assert_eq!(first.by_resolver[0].fetched, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let entity = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entity.status, crate::model::knowledge_entity::Status::Identified);
        assert_eq!(entity.confidence, Some(0.9));

        // Second run: the entity now has an ontology claim, so it is not even a candidate.
        let second = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers,
                calls: calls.clone(),
            }))
            .resolve_pending()
            .await
            .expect("resolve");
        assert_eq!(second.candidates, 0, "{second:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second fetch");
    }

    /// A second authority sees an entity the first one answered, and both claims survive.
    #[tokio::test]
    async fn two_authorities_may_assert_one_entity() {
        db_or_skip!(db);
        let entity = seed(&db, "Train Dreams").await;
        let answers = HashMap::from([("Train Dreams".to_string(), identified("Q1"))]);

        let engine = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers,
                calls: Arc::new(AtomicUsize::new(0)),
            }))
            .with(Box::new(Other));
        let report = engine.resolve_pending().await.expect("resolve");

        assert_eq!(report.claims, 2, "{report:?}");
        assert_eq!(report.by_resolver.len(), 2, "{report:?}");
        assert_eq!(report.by_resolver[0].provider, "wikidata");
        assert_eq!(report.by_resolver[1].provider, "imdb");
        assert_eq!(
            report.by_resolver[1].candidates, 1,
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

        let report = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Many))
            .resolve(std::slice::from_ref(&entity))
            .await
            .expect("resolve");

        assert_eq!(report.claims, MAX_CLAIMS_PER_ENTITY, "{report:?}");
        assert_eq!(report.by_resolver[0].dropped, 2, "{report:?}");
        assert_eq!(
            model::knowledge_claims::Entity::find()
                .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
                .count(&db.conn)
                .await
                .unwrap(),
            MAX_CLAIMS_PER_ENTITY as u64
        );
    }

    /// A forced re-resolution may replace what the same authority said before, but only in the
    /// scheme it answered.
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

        let report = ResolutionEngine::new(
            &db,
            Options {
                force: true,
                replace: true,
                ..Default::default()
            },
        )
        .with(Box::new(Other))
        .resolve(std::slice::from_ref(&entity))
        .await
        .expect("resolve");

        assert_eq!(report.claims, 1, "{report:?}");
        assert_eq!(report.by_resolver[0].replaced, 1, "{report:?}");

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

        let first = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers: HashMap::new(),
                calls: calls.clone(),
            }))
            .resolve_pending()
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
            .resolve_pending()
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
        assert_eq!(entity.status, crate::model::knowledge_entity::Status::Provisional);
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
        .resolve_pending()
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
        assert_eq!(entity.status, crate::model::knowledge_entity::Status::Identified, "{entity:?}");

        // The miss is left alone, which is the whole point: offline builds what the cache can and
        // stops there.
        let entity = model::knowledge_entity::Entity::find_by_id(uncached)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entity.status, crate::model::knowledge_entity::Status::Provisional, "{entity:?}");
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

        let report = ResolutionEngine::new(&db, Options::default())
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
        ResolutionEngine::new(&db, Options::default())
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

        let report = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Fake {
                answers,
                calls: Arc::new(AtomicUsize::new(0)),
            }))
            .resolve_pending()
            .await
            .expect("resolve");

        assert_eq!(report.claims, 2, "{report:?}");

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
        let error = ResolutionEngine::new(&db, Options::default())
            .with(Box::new(Reserved))
            .resolve_pending()
            .await
            .expect_err("the reserved id must be refused");
        assert!(error.to_string().contains("reserved"), "{error}");
    }
}
