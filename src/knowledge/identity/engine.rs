//! The engine: the authority contract, and the run loop that drives it.
//!
//! A resolver owns its strategy — one request or a series of them, one pass or several. `Gateway` is
//! its only door to the world: the gateway consults the cache first, paces the call and charges the
//! budget. The engine selects the entities, gives each resolver a batch, and stores what comes back.
//!
//! A resolver's answer is one provider's complete statement about the entities it names. The engine
//! therefore replaces that provider's annotation set for every entity that got an answer. An
//! abstention leaves the old annotations alone.

use std::collections::{BTreeMap, HashSet};

use crate::fetch::{self, Allowance, FetchCache, Gateway, Refusal};
use crate::knowledge::KnowledgeGraph;
use crate::knowledge::evidence::{Evidence, Identity};
use crate::model;

use super::RESERVED_PROVIDER_IDS;

/// One authority and its strategy for identifying entities.
///
/// Named for what it returns: an [`Identity`](crate::knowledge::evidence::Identity) per entity, not a
/// verdict about a pair or a document. A future provider answering a different question — an
/// attestation, say — is a different trait, not a second implementor of this one.
///
/// `resolve` takes a *batch* of entities rather than one: batching across entities is the difference
/// between a hundred transport calls for a corpus walk and five thousand, and only the resolver knows
/// how to group its own requests.
#[async_trait::async_trait]
pub trait IdentityResolver: Send + Sync {
    /// The authority's id: the `provider` on every request and every identity it writes. Reserved
    /// ids are refused by [`IdentityEngine::new`].
    fn id(&self) -> &'static str;

    /// The complete set of identities this authority asserts for each entity it answers about.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)>;
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Ask a resolver again even when it already annotated the entity. `--entity` sets this;
    /// without it, the per-provider filter makes an entity-scoped run empty.
    pub include_already_annotated: bool,
    /// Report what a positive answer would write, without writing or projecting it.
    pub dry_run: bool,
}

/// One identity a dry run would write, as the report's wording: entity, provider and key.
///
/// Not `Identity`: that is the resolver's input and carries the full metadata, no entity or
/// provider, and an `f32` confidence. This is the minimal `Eq` row a report can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunIdentity {
    pub entity_id: i32,
    pub provider: String,
    pub scheme: String,
    pub external_id: String,
    pub method: Option<String>,
}

/// One resolution run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolutionReport {
    pub offline: bool,
    /// No identities were written or projected; the counts are what the run would do. Provider
    /// answers may still populate `fetch_cache`.
    pub dry_run: bool,

    pub considered: usize,
    pub fetch_traffic: fetch::Stats,

    pub entities_resolved: usize,
    pub identities_written: usize,
    pub identities_vetoed: usize,
    pub identities_replaced: usize,

    pub by_provider: Vec<ProviderReport>,
    /// What a dry run would write; empty for a real run.
    pub preview: Vec<DryRunIdentity>,
    /// The run-wide budget refusal, or a provider's rate limit, for the report.
    pub refusal: Option<Refusal>,
}

/// What one provider did in one run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProviderReport {
    pub provider: String,
    pub batch: usize,
    pub fetch_traffic: fetch::Stats,

    pub identities_written: usize,
    pub identities_vetoed: usize,
    /// Old annotations this provider owned that the run replaced.
    pub identities_replaced: usize,
}

/// The harness: it holds the resolvers, invokes them, and stores their output.
pub struct IdentityEngine {
    graph: KnowledgeGraph,
    options: Options,
    resolvers: Vec<Box<dyn IdentityResolver>>,
}

impl IdentityEngine {
    pub fn new(
        graph: KnowledgeGraph,
        options: Options,
        resolvers: Vec<Box<dyn IdentityResolver>>,
    ) -> anyhow::Result<Self> {
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
        cache: &FetchCache,
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
        let dry_run = self.options.dry_run;
        let mut run = ResolutionRun::new(self, &entities, &gateway).await?;
        run.collect(&entities).await?;
        if dry_run {
            run.preview();
        } else {
            run.apply().await?;
        }
        Ok(run.finish())
    }
}

/// The mutable state of one engine-level `resolve`.
struct ResolutionRun<'e, 'g, 'a> {
    engine: &'e IdentityEngine,
    gateway: &'g Gateway<'a>,
    evidence: &'e Evidence,
    vetoes: HashSet<(i32, String, String)>,
    report: ResolutionReport,
    provider_reports: Vec<ProviderReport>,
    proposed: BTreeMap<i32, Vec<(usize, Identity)>>,
    resolved: HashSet<i32>,
}

impl<'e, 'g, 'a> ResolutionRun<'e, 'g, 'a> {
    async fn new(
        engine: &'e IdentityEngine,
        entities: &[model::knowledge_entity::Model],
        gateway: &'g Gateway<'a>,
    ) -> anyhow::Result<Self> {
        // Loaded once, not per identity: a veto asked during admission is an identity never
        // written; one that is only replayed is asserted and then withdrawn.
        let vetoes = engine.graph.vetoed().await?;
        Ok(Self {
            engine,
            gateway,
            evidence: engine.graph.evidence(),
            vetoes,
            report: ResolutionReport {
                considered: entities.len(),
                offline: gateway.offline(),
                dry_run: engine.options.dry_run,
                ..Default::default()
            },
            provider_reports: Vec::new(),
            proposed: BTreeMap::new(),
            resolved: HashSet::new(),
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

            let already_annotated = if self.engine.options.include_already_annotated {
                HashSet::new()
            } else {
                self.evidence
                    .selection()
                    .annotated_by(resolver.id(), &entity_ids)
                    .await?
            };
            let batch: Vec<model::knowledge_entity::Model> = entities
                .iter()
                .filter(|entity| !already_annotated.contains(&entity.id))
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
                for (knowledge_entity_id, identity) in resolver.resolve(&batch, self.gateway).await
                {
                    if !batch_ids.contains(&knowledge_entity_id) {
                        tracing::warn!(
                            provider = resolver.id(),
                            knowledge_entity_id,
                            "resolver returned an identity outside its batch"
                        );
                        continue;
                    }
                    self.proposed
                        .entry(knowledge_entity_id)
                        .or_default()
                        .push((index, identity));
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
        let proposed = std::mem::take(&mut self.proposed);
        for (knowledge_entity_id, proposals) in proposed {
            // Entity-first apply: every authority's answer for an entity lands before its
            // projection, so status and confidence see all of them.
            for (resolver_index, identities) in group_by_resolver(proposals) {
                if identities.is_empty() {
                    continue;
                }
                let provider = self.engine.resolvers[resolver_index].id();
                // A refused provider's output is partial, not a complete statement: keep what the
                // graph already holds rather than replacing it with half an answer.
                if self.gateway.refusal_for(provider).is_some() {
                    continue;
                }
                let write = self
                    .engine
                    .graph
                    .replace_provider_identities(
                        knowledge_entity_id,
                        provider,
                        &identities,
                        &self.vetoes,
                    )
                    .await?;
                self.report.identities_written += write.written;
                self.report.identities_vetoed += write.vetoed;
                self.report.identities_replaced += write.removed;
                let provider_report = &mut self.provider_reports[resolver_index];
                provider_report.identities_written += write.written;
                provider_report.identities_vetoed += write.vetoed;
                provider_report.identities_replaced += write.removed;
                if write.written > 0 {
                    self.resolved.insert(knowledge_entity_id);
                }
            }

            // A later `proximity auto-merge` picks a survivor by the projected `confidence`, so an
            // entity that just earned annotations is projected here — after all authorities spoke.
            self.engine
                .graph
                .update_projection(knowledge_entity_id)
                .await?;
        }
        Ok(())
    }

    /// Report the positive answers without writing or projecting. The semantics are the write's:
    /// whatever each provider holds now would be replaced by this set.
    fn preview(&mut self) {
        let proposed = std::mem::take(&mut self.proposed);
        for (entity_id, proposals) in proposed {
            for (resolver_index, identities) in group_by_resolver(proposals) {
                let provider = self.engine.resolvers[resolver_index].id();
                if self.gateway.refusal_for(provider).is_some() {
                    continue;
                }
                let (keep, vetoed) = crate::knowledge::evidence::filter_identities(
                    entity_id,
                    &identities,
                    &self.vetoes,
                );
                let written = keep.len();
                self.report.identities_written += written;
                self.report.identities_vetoed += vetoed;
                let provider_report = &mut self.provider_reports[resolver_index];
                provider_report.identities_written += written;
                provider_report.identities_vetoed += vetoed;
                if written > 0 {
                    self.resolved.insert(entity_id);
                }
                self.report
                    .preview
                    .extend(keep.into_iter().map(|identity| DryRunIdentity {
                        entity_id,
                        provider: provider.to_string(),
                        scheme: identity.scheme,
                        external_id: identity.external_id,
                        method: identity.method,
                    }));
            }
        }
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

fn group_by_resolver(proposals: Vec<(usize, Identity)>) -> BTreeMap<usize, Vec<Identity>> {
    let mut by_resolver: BTreeMap<usize, Vec<Identity>> = BTreeMap::new();
    for (resolver_index, identity) in proposals {
        by_resolver
            .entry(resolver_index)
            .or_default()
            .push(identity);
    }
    by_resolver
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use super::*;
    use crate::test_support::{db_or_skip, fixtures};

    fn identity(external_id: &str) -> Identity {
        Identity {
            scheme: "wikidata".to_string(),
            external_id: external_id.to_string(),
            method: Some("exact_label".to_string()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: None,
        }
    }

    struct Fixed(Vec<(i32, Identity)>);

    #[async_trait::async_trait]
    impl IdentityResolver for Fixed {
        fn id(&self) -> &'static str {
            "wikidata"
        }

        async fn resolve(
            &self,
            _entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(i32, Identity)> {
            self.0.clone()
        }
    }

    async fn run(
        db: &crate::database::DbHandle,
        entity: &model::knowledge_entity::Model,
        resolver: Fixed,
        dry_run: bool,
    ) -> ResolutionReport {
        let cache = FetchCache::new(&db.conn);
        IdentityEngine::new(
            KnowledgeGraph::new(db),
            Options {
                include_already_annotated: true,
                dry_run,
            },
            vec![Box::new(resolver)],
        )
        .unwrap()
        .resolve(std::slice::from_ref(entity), &cache, None)
        .await
        .unwrap()
    }

    async fn stored(db: &crate::database::DbHandle, id: i32) -> Vec<String> {
        model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(id))
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.external_id)
            .collect()
    }

    /// A positive answer is the provider's complete statement: its old rows go.
    #[tokio::test]
    async fn a_positive_answer_replaces_the_providers_rows() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::identity(&db, id, "wikidata", "Q1", "wikidata", None, None).await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        let report = run(&db, &entity, Fixed(vec![(id, identity("Q2"))]), false).await;

        assert_eq!(report.identities_written, 1);
        assert_eq!(report.identities_replaced, 1);
        assert_eq!(stored(&db, id).await, vec!["Q2".to_string()]);
    }

    /// An abstention is not a complete statement: the graph keeps what it had.
    #[tokio::test]
    async fn an_empty_answer_leaves_the_old_rows() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::identity(&db, id, "wikidata", "Q1", "wikidata", None, None).await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        let report = run(&db, &entity, Fixed(Vec::new()), false).await;

        assert_eq!(report.identities_written, 0);
        assert_eq!(report.identities_replaced, 0);
        assert_eq!(stored(&db, id).await, vec!["Q1".to_string()]);
    }

    /// A dry run reports the replacement set and changes nothing.
    #[tokio::test]
    async fn a_dry_run_reports_the_replacement_without_writing() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::identity(&db, id, "wikidata", "Q1", "wikidata", None, None).await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        let report = run(&db, &entity, Fixed(vec![(id, identity("Q2"))]), true).await;

        assert!(report.dry_run);
        assert_eq!(report.identities_written, 1);
        assert_eq!(report.identities_replaced, 0);
        assert_eq!(report.entities_resolved, 1);
        assert_eq!(report.preview.len(), 1, "{report:?}");
        assert_eq!(report.preview[0].external_id, "Q2");
        assert_eq!(
            stored(&db, id).await,
            vec!["Q1".to_string()],
            "the graph still holds what it held"
        );
    }
}
