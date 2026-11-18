//! The first end-to-end use cases: link illuminations into the v2 graph and
//! identify entities with Wikidata.
//!
//! The "helper" structs below are external executors which leverage the
//! heuristic linker and identifier to perform the end-to-end graph
//! construction operations

use std::collections::{BTreeMap, HashSet};

use crate::database::DbHandle;
use crate::fetch::{self, Allowance, FetchCache, Gateway};
use crate::knowledge::blocker::SqlBlocker;
use crate::knowledge::graph::KnowledgeGraph;
use crate::knowledge::heuristic::HeuristicIdentifier;
use crate::knowledge::heuristic::HeuristicLinker;
use crate::knowledge::identity::{Identity, Provider};
use crate::knowledge::ids::{EntityOriginId, ObservationId};
use crate::knowledge::mutation::GraphMutation;
use crate::knowledge::wikidata::ID;
use crate::model;

/// One illumination's link.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkReport {
    pub considered: usize,
    pub already_linked: usize,
    pub created: usize,
    pub attached: usize,
    pub left_pending: usize,
}

/// Every illumination that still had work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkAllReport {
    pub illuminations: usize,
    pub considered: usize,
    pub already_linked: usize,
    pub created: usize,
    pub attached: usize,
    pub left_pending: usize,
}

/// The external executor: linker and graph.
#[derive(Clone)]
pub struct LinkHelper {
    graph: KnowledgeGraph,
    blocker: SqlBlocker,
    linker: HeuristicLinker,
}

impl LinkHelper {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            graph: KnowledgeGraph::new(db),
            blocker: SqlBlocker::new(db),
            linker: HeuristicLinker::new(),
        }
    }

    /// Link one illumination: decide, apply, project.
    pub async fn link_illumination(
        &self,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<LinkReport> {
        // An unloaded relation would read as an illumination with no observations, and linking
        // nothing would look exactly like success.
        anyhow::ensure!(
            illumination.observations.is_loaded(),
            "illumination {} was handed to link_illumination with no observations loaded; fetch it \
             with Entity::load().with(model::user_observed_entity::Entity)",
            illumination.id
        );

        let observations: Vec<model::user_observed_entity::Model> = illumination
            .observations
            .iter()
            .map(|row| row.clone().into())
            .collect();
        let considered = observations.len();
        let pending: Vec<model::user_observed_entity::Model> = observations
            .into_iter()
            .filter(|row| row.knowledge_entity_id.is_none())
            .collect();
        let already_linked = considered - pending.len();

        let mutations = self.linker.plan(&self.blocker, &pending).await?;
        let created = mutations
            .iter()
            .filter(|mutation| matches!(mutation, GraphMutation::CreateEntity { .. }))
            .count();
        let attached = mutations.len() - created;

        if !mutations.is_empty() {
            let report = self.graph.proc(&mutations).await?;
            anyhow::ensure!(
                report.refused.is_none(),
                "linking refused: {:?}",
                report.refused
            );
        }

        Ok(LinkReport {
            considered,
            already_linked,
            created,
            attached,
            left_pending: pending.len().saturating_sub(mutations.len()),
        })
    }

    /// Link every illumination that still has an unlinked observation.
    ///
    /// One illumination, one `proc`, one projection pass — the same boundary as v1. The order is by
    /// illumination id, because linking one changes what a later one finds.
    pub async fn link_all_pending(&self) -> anyhow::Result<LinkAllReport> {
        let illuminations = self.graph.selector().awaiting_link().await?;

        let mut report = LinkAllReport::default();
        for illumination in &illuminations {
            let part = self.link_illumination(illumination).await?;
            report.illuminations += 1;
            report.considered += part.considered;
            report.already_linked += part.already_linked;
            report.created += part.created;
            report.attached += part.attached;
            report.left_pending += part.left_pending;
        }
        Ok(report)
    }
}

#[derive(Debug, Clone, Default)]
pub struct IdentificationOptions {
    /// Ask Wikidata again even when it already annotated the entity. `--entity` sets this;
    /// without it, the already-annotated filter makes an entity-scoped run empty.
    pub include_already_annotated: bool,
    /// Report what a positive answer would write, without writing or projecting it.
    pub dry_run: bool,
}

/// One identity a dry run would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunIdentity {
    pub entity_id: i32,
    pub scheme: String,
    pub external_id: String,
    pub method: Option<String>,
}

/// One identification run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct IdentificationReport {
    pub considered: usize,
    /// Entities sent to Wikidata after the already-annotated filter.
    pub batch: usize,
    pub fetch_traffic: fetch::Stats,

    pub entities_identified: usize,
    pub identities_written: usize,
    pub identities_vetoed: usize,
    pub identities_replaced: usize,

    /// What a dry run would write; empty for a real run.
    pub preview: Vec<DryRunIdentity>,
    /// The run-wide budget refusal, or Wikidata's rate limit, for the report.
    pub refusal: Option<fetch::Refusal>,
}

/// The identification executor: the heuristic provider and the graph.
pub struct IdentifyHelper {
    graph: KnowledgeGraph,
    options: IdentificationOptions,
    identifier: HeuristicIdentifier,
}

impl IdentifyHelper {
    pub fn new(db: &DbHandle, options: IdentificationOptions) -> anyhow::Result<Self> {
        Ok(Self {
            graph: KnowledgeGraph::new(db),
            options,
            identifier: HeuristicIdentifier::new()?,
        })
    }

    /// Identify exactly the entities in scope; selection is the caller's job.
    pub async fn identify(
        &self,
        entities: &[model::knowledge_entity::Model],
        cache: &FetchCache,
        allowance: Option<&Allowance>,
    ) -> anyhow::Result<IdentificationReport> {
        // The caller may hand in a tombstone or a duplicate; neither is a question to ask.
        let mut seen = HashSet::new();
        let entities: Vec<model::knowledge_entity::Model> = entities
            .iter()
            .filter(|entity| entity.merged_into_id.is_none() && seen.insert(entity.id))
            .cloned()
            .collect();

        let gateway = Gateway::new(Some(cache), allowance, true);
        let selector = self.graph.selector();
        let vetoes = selector.verdicts().vetoed_keys().await?;

        let mut report = IdentificationReport {
            considered: entities.len(),
            ..Default::default()
        };
        let mut proposed: BTreeMap<i32, Vec<Identity>> = BTreeMap::new();
        let mut identified = HashSet::new();

        if gateway.refusal_for(ID).is_none() {
            let entity_ids: Vec<i32> = entities.iter().map(|entity| entity.id).collect();
            let already_annotated = if self.options.include_already_annotated {
                HashSet::new()
            } else {
                selector.identified_by(ID, &entity_ids).await?
            };
            let batch: Vec<model::knowledge_entity::Model> = entities
                .iter()
                .filter(|entity| !already_annotated.contains(&entity.id))
                .cloned()
                .collect();
            let batch_ids: HashSet<i32> = batch.iter().map(|entity| entity.id).collect();

            report.batch = batch.len();
            if !batch.is_empty() {
                for (knowledge_entity_id, identity) in
                    self.identifier.identify_batch(&batch, &gateway).await
                {
                    if !batch_ids.contains(&knowledge_entity_id) {
                        tracing::warn!(
                            provider = ID,
                            knowledge_entity_id,
                            "Wikidata returned an identity outside its batch"
                        );
                        continue;
                    }
                    proposed
                        .entry(knowledge_entity_id)
                        .or_default()
                        .push(identity);
                }
            }
            if self.options.dry_run {
                for (entity_id, identities) in &proposed {
                    let (keep, vetoed) = filter_identities(*entity_id, identities, &vetoes);
                    let written = keep.len();
                    report.identities_written += written;
                    report.identities_vetoed += vetoed;
                    if written > 0 {
                        identified.insert(*entity_id);
                    }
                    report
                        .preview
                        .extend(keep.into_iter().map(|identity| DryRunIdentity {
                            entity_id: *entity_id,
                            scheme: identity.scheme,
                            external_id: identity.external_id,
                            method: identity.method,
                        }));
                }
            } else {
                let mut mutations = Vec::new();
                for (entity_id, identities) in &proposed {
                    let Some(entity) = entities.iter().find(|entity| entity.id == *entity_id)
                    else {
                        continue;
                    };
                    let origin = entity.origin_observation_id;
                    let subject = EntityOriginId(ObservationId(origin));
                    let (keep, vetoed) = filter_identities(*entity_id, identities, &vetoes);
                    let existing = selector.identity_keys(*entity_id, ID).await?;
                    let removed = existing.len();
                    for key in existing {
                        mutations.push(GraphMutation::RetractIdentity {
                            subject,
                            provider: Provider(ID.to_string()),
                            key,
                        });
                    }
                    let written = keep.len();
                    for mut identity in keep {
                        identity.provider = Provider(ID.to_string());
                        mutations.push(GraphMutation::Identify { subject, identity });
                    }

                    report.identities_written += written;
                    report.identities_vetoed += vetoed;
                    report.identities_replaced += removed;
                    if written > 0 {
                        identified.insert(*entity_id);
                    }
                }
                if !mutations.is_empty() {
                    self.graph.proc(&mutations).await?;
                }
            }
        }

        report.entities_identified = identified.len();
        report.fetch_traffic = gateway.stats();
        report.refusal = gateway.refusal();
        Ok(report)
    }

    /// Identify every entity still pending for Wikidata.
    pub async fn identify_all_pending(
        &self,
        cache: &FetchCache,
        allowance: Option<&Allowance>,
        limit: Option<usize>,
        newest_first: bool,
    ) -> anyhow::Result<IdentificationReport> {
        let mut entities = self
            .graph
            .selector()
            .awaiting_identity(&[ID], None, newest_first)
            .await?;
        if let Some(limit) = limit {
            entities.truncate(limit);
        }

        // An online corpus pass is the maintenance moment: drop answers that aged out long ago. A
        // per-illumination identification does not do this — it must stay quick.
        if allowance.is_some() {
            let pruned = cache.prune().await?;
            if pruned > 0 {
                tracing::info!(pruned, "Pruned expired fetch cache rows");
            }
        }

        self.identify(&entities, cache, allowance).await
    }
}

/// The identities a write would keep: deduped by key and filtered by a standing veto.
fn filter_identities(
    entity_id: i32,
    identities: &[Identity],
    vetoes: &HashSet<(i32, String, String)>,
) -> (Vec<Identity>, usize) {
    let mut seen = HashSet::new();
    let mut keep = Vec::new();
    let mut vetoed = 0;
    for identity in identities {
        if !seen.insert((identity.scheme.clone(), identity.external_id.clone())) {
            continue;
        }
        if vetoes.contains(&(
            entity_id,
            identity.scheme.clone(),
            identity.external_id.clone(),
        )) {
            vetoed += 1;
            continue;
        }
        keep.push(identity.clone());
    }
    (keep, vetoed)
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    use super::*;
    use crate::fetch::cache::FetchCache;
    use crate::knowledge::identity::IdentityKey;
    use crate::knowledge::mutation::Cause;
    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn links_two_illuminations_that_share_a_handle() {
        db_or_skip!(db);
        let first = fixtures::Capture::new(&db, "k2_link_first").await;
        let second = fixtures::Capture::new(&db, "k2_link_second").await;

        fixtures::observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            Some("instagram"),
            Some("traindreams"),
        )
        .await;
        fixtures::observe(
            &db,
            &second,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            Some("instagram"),
            Some("traindreams"),
        )
        .await;

        let report = LinkHelper::new(&db).link_all_pending().await.unwrap();

        assert_eq!(report.illuminations, 2);
        assert_eq!(report.created, 1);
        assert_eq!(report.attached, 1);
        assert_eq!(report.left_pending, 0);
        assert_eq!(
            model::knowledge_entity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            model::user_observed_entity::Entity::find()
                .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
                .count(&db.conn)
                .await
                .unwrap(),
            2
        );
    }

    fn identity(external_id: &str) -> Identity {
        Identity {
            provider: Provider(ID.into()),
            scheme: "fake".into(),
            external_id: external_id.into(),
            method: Some("test".into()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: None,
        }
    }

    fn identifier(db: &DbHandle, dry_run: bool) -> IdentifyHelper {
        let mut helper = IdentifyHelper::new(
            db,
            IdentificationOptions {
                include_already_annotated: true,
                dry_run,
            },
        )
        .expect("the identifier builds");
        helper.identifier = HeuristicIdentifier::with_test_identity(identity("Q2"))
            .expect("the test identifier builds");
        helper
    }

    async fn attest(db: &DbHandle, entity: i32, external_id: &str) {
        let row = model::knowledge_entity::Entity::find_by_id(entity)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the fixture entity exists");
        let origin = row.origin_observation_id;
        KnowledgeGraph::new(db)
            .proc(&[GraphMutation::Identify {
                subject: EntityOriginId(ObservationId(origin)),
                identity: identity(external_id),
            }])
            .await
            .unwrap();
    }

    async fn keys(db: &DbHandle, entity: i32) -> Vec<String> {
        model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity))
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.external_id)
            .collect()
    }

    #[tokio::test]
    async fn identify_all_pending_skips_entities_wikidata_already_answered() {
        db_or_skip!(db);
        let annotated = fixtures::knowledge_entity_with_member(&db, "Already Known").await;
        let pending = fixtures::knowledge_entity_with_member(&db, "Still Pending").await;
        fixtures::identity(
            &db,
            annotated,
            "wikidata",
            "Q9",
            "wikidata",
            None,
            Some(0.9),
        )
        .await;

        let mut helper = IdentifyHelper::new(&db, IdentificationOptions::default()).unwrap();
        helper.identifier = HeuristicIdentifier::with_test_identity(identity("Q2"))
            .expect("the test identifier builds");
        let cache = FetchCache::new(&db.conn);
        let report = helper
            .identify_all_pending(&cache, None, None, false)
            .await
            .unwrap();

        assert_eq!(report.considered, 1);
        assert_eq!(report.batch, 1);
        assert_eq!(keys(&db, annotated).await, ["Q9"]);
        assert_eq!(keys(&db, pending).await, ["Q2"]);
    }

    #[tokio::test]
    async fn a_new_answer_retracts_the_old_provider_set_and_attests_the_new_one() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attest(&db, entity, "Q1").await;

        let cache = FetchCache::new(&db.conn);
        let report = identifier(&db, false)
            .identify(
                &[model::knowledge_entity::Entity::find_by_id(entity)
                    .one(&db.conn)
                    .await
                    .unwrap()
                    .unwrap()],
                &cache,
                None,
            )
            .await
            .unwrap();

        assert_eq!(report.identities_written, 1, "{report:?}");
        assert_eq!(report.identities_replaced, 1, "{report:?}");
        assert_eq!(report.entities_identified, 1, "{report:?}");
        assert_eq!(keys(&db, entity).await, ["Q2"]);
    }

    #[tokio::test]
    async fn a_dry_run_previews_without_writing() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attest(&db, entity, "Q1").await;

        let cache = FetchCache::new(&db.conn);
        let report = identifier(&db, true)
            .identify(
                &[model::knowledge_entity::Entity::find_by_id(entity)
                    .one(&db.conn)
                    .await
                    .unwrap()
                    .unwrap()],
                &cache,
                None,
            )
            .await
            .unwrap();

        assert_eq!(report.identities_written, 1, "{report:?}");
        assert_eq!(
            report
                .preview
                .first()
                .map(|preview| preview.external_id.as_str()),
            Some("Q2")
        );
        assert_eq!(keys(&db, entity).await, ["Q1"]);
    }

    #[tokio::test]
    async fn a_standing_veto_is_counted_and_its_old_row_is_retracted() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attest(&db, entity, "Q1").await;
        let row = model::knowledge_entity::Entity::find_by_id(entity)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let origin = row.origin_observation_id;
        KnowledgeGraph::new(&db)
            .proc(&[GraphMutation::VetoIdentity {
                subject: EntityOriginId(ObservationId(origin)),
                key: IdentityKey {
                    scheme: "fake".into(),
                    external_id: "Q2".into(),
                },
                cause: Cause::Repair,
                why: "test veto".into(),
                record_verdict: true,
            }])
            .await
            .unwrap();

        let cache = FetchCache::new(&db.conn);
        let report = identifier(&db, false)
            .identify(
                &[model::knowledge_entity::Entity::find_by_id(entity)
                    .one(&db.conn)
                    .await
                    .unwrap()
                    .unwrap()],
                &cache,
                None,
            )
            .await
            .unwrap();

        assert_eq!(report.identities_vetoed, 1, "{report:?}");
        assert_eq!(report.identities_written, 0, "{report:?}");
        assert_eq!(report.identities_replaced, 1, "{report:?}");
        assert!(keys(&db, entity).await.is_empty());
    }

    #[tokio::test]
    async fn the_projection_pass_keeps_the_blocker_current() {
        db_or_skip!(db);
        let first = fixtures::Capture::new(&db, "k2_index_first").await;
        let second = fixtures::Capture::new(&db, "k2_index_second").await;

        fixtures::observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            None,
            None,
        )
        .await;
        fixtures::observe(
            &db,
            &second,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            None,
            None,
        )
        .await;

        let report = LinkHelper::new(&db).link_all_pending().await.unwrap();

        assert_eq!(report.created, 1);
        assert_eq!(report.attached, 1);
        assert_eq!(
            model::knowledge_entity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1
        );
    }
}
