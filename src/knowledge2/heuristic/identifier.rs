//! The harness: select, ask, turn answers into mutations, report.

use std::collections::{BTreeMap, HashSet};

use crate::fetch::{self, Allowance, FetchCache, Gateway};
use crate::knowledge2::mutation::GraphMutation;
use crate::knowledge2::graph::KnowledgeGraph2;
use crate::knowledge2::identity::{Identity, Provider};
use crate::knowledge2::ids::{EntityOriginId, ObservationId};
use crate::knowledge2::wikidata::ID;
use crate::model;

pub use super::resolution::{DryRunIdentity, Options, ProviderReport, ResolutionReport};
use super::resolution::Wikidata;

/// Transport calls a plain resolve is allowed to make when the caller does not choose a budget.
pub const DEFAULT_MAX_REQUESTS: usize = 200;

/// The heuristic identifier: Wikidata, the cache gateway and the graph.
pub struct HeuristicIdentifier {
    graph: KnowledgeGraph2,
    options: Options,
    wikidata: Wikidata,
    #[cfg(test)]
    test_identity: Option<Identity>,
}

impl HeuristicIdentifier {
    /// The identifier over Wikidata.
    pub fn new(graph: &KnowledgeGraph2, options: Options) -> anyhow::Result<Self> {
        Ok(Self {
            graph: graph.clone(),
            options,
            wikidata: Wikidata::new()?,
            #[cfg(test)]
            test_identity: None,
        })
    }

    #[cfg(test)]
    fn with_test_identity(
        graph: &KnowledgeGraph2,
        options: Options,
        identity: Identity,
    ) -> anyhow::Result<Self> {
        let mut identifier = Self::new(graph, options)?;
        identifier.test_identity = Some(identity);
        Ok(identifier)
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
        let selector = self.graph.selector();
        let vetoes = selector.verdicts().vetoed_keys().await?;

        let mut report = ResolutionReport {
            considered: entities.len(),
            offline: gateway.offline(),
            dry_run: self.options.dry_run,
            ..Default::default()
        };
        let mut provider_report = ProviderReport {
            provider: ID.to_string(),
            ..Default::default()
        };
        let mut proposed: BTreeMap<i32, Vec<Identity>> = BTreeMap::new();
        let mut resolved = HashSet::new();

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

            provider_report.batch = batch.len();
            let before = gateway.stats();
            if !batch.is_empty() {
                for (knowledge_entity_id, identity) in self.identities(&batch, &gateway).await {
                    if !batch_ids.contains(&knowledge_entity_id) {
                        tracing::warn!(
                            provider = ID,
                            knowledge_entity_id,
                            "Wikidata returned an identity outside its batch"
                        );
                        continue;
                    }
                    proposed.entry(knowledge_entity_id).or_default().push(identity);
                }
            }
            let after = gateway.stats();
            provider_report.fetch_traffic = fetch::Stats {
                requested: after.requested.saturating_sub(before.requested),
                cache_hits: after.cache_hits.saturating_sub(before.cache_hits),
                fetched: after.fetched.saturating_sub(before.fetched),
                failed: after.failed.saturating_sub(before.failed),
            };

            if self.options.dry_run {
                for (entity_id, identities) in &proposed {
                    let (keep, vetoed) = filter_identities(*entity_id, identities, &vetoes);
                    let written = keep.len();
                    report.identities_written += written;
                    report.identities_vetoed += vetoed;
                    provider_report.identities_written += written;
                    provider_report.identities_vetoed += vetoed;
                    if written > 0 {
                        resolved.insert(*entity_id);
                    }
                    report.preview.extend(keep.into_iter().map(|identity| DryRunIdentity {
                        entity_id: *entity_id,
                        provider: ID.to_string(),
                        scheme: identity.scheme,
                        external_id: identity.external_id,
                        method: identity.method,
                    }));
                }
            } else {
                let mut mutations = Vec::new();
                for (entity_id, identities) in &proposed {
                    let Some(entity) = entities.iter().find(|entity| entity.id == *entity_id) else {
                        continue;
                    };
                    let Some(origin) = entity.origin_observation_id else {
                        tracing::warn!(knowledge_entity_id = *entity_id, "entity has no origin");
                        continue;
                    };
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
                    provider_report.identities_written += written;
                    provider_report.identities_vetoed += vetoed;
                    provider_report.identities_replaced += removed;
                    if written > 0 {
                        resolved.insert(*entity_id);
                    }
                }
                if !mutations.is_empty() {
                    self.graph.proc(&mutations).await?;
                }
            }
        }

        report.entities_resolved = resolved.len();
        report.by_provider = vec![provider_report];
        report.fetch_traffic = gateway.stats();
        report.refusal = gateway.refusal();
        Ok(report)
    }

    /// Ask the one authority. Tests substitute a canned identity without a transport.
    async fn identities(
        &self,
        batch: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)> {
        #[cfg(test)]
        if let Some(identity) = &self.test_identity {
            return batch.iter().map(|entity| (entity.id, identity.clone())).collect();
        }
        self.wikidata.resolve(batch, gateway).await
    }
}

impl HeuristicIdentifier {
    /// Resolve every entity still pending for Wikidata.
    ///
    /// The engine receives the whole pending set in one call, so the wavefront still fills its
    /// batches. This is the rebuild/drain path; a live capture resolves through the service API's
    /// illumination read, which also reports whether linking finished.
    pub async fn resolve_all_pending(
        &self,
        cache: &FetchCache,
        allowance: Option<&Allowance>,
        limit: Option<usize>,
        newest_first: bool,
    ) -> anyhow::Result<ResolutionReport> {
        let mut entities = self
            .graph
            .selector()
            .awaiting_identity(&[ID], None, newest_first)
            .await?;
        if let Some(limit) = limit {
            entities.truncate(limit);
        }

        // An online corpus pass is the maintenance moment: drop answers that aged out long ago. A
        // per-illumination resolve does not do this — it must stay quick.
        if allowance.is_some() {
            let pruned = cache.prune().await?;
            if pruned > 0 {
                tracing::info!(pruned, "Pruned expired fetch cache rows");
            }
        }

        self.resolve(&entities, cache, allowance).await
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
    use super::*;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use crate::database::DbHandle;
    use crate::knowledge2::identity::IdentityKey;
    use crate::knowledge2::mutation::Cause;
    use crate::test_support::{db_or_skip, fixtures};

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

    async fn resolver(db: &DbHandle, dry_run: bool) -> HeuristicIdentifier {
        HeuristicIdentifier::with_test_identity(
            &KnowledgeGraph2::new(db),
            Options {
                include_already_annotated: true,
                dry_run,
            },
            identity("Q2"),
        )
        .expect("the test identifier builds")
    }

    async fn attest(db: &DbHandle, entity: i32, external_id: &str) {
        let row = model::knowledge_entity::Entity::find_by_id(entity)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the fixture entity exists");
        let origin = row.origin_observation_id.expect("the fixture has an origin");
        KnowledgeGraph2::new(db)
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
    async fn a_new_answer_retracts_the_old_provider_set_and_attests_the_new_one() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attest(&db, entity, "Q1").await;

        let cache = FetchCache::new(&db.conn);
        let report = resolver(&db, false)
            .await
            .resolve(
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
        assert_eq!(report.entities_resolved, 1, "{report:?}");
        assert_eq!(keys(&db, entity).await, ["Q2"]);
    }

    #[tokio::test]
    async fn a_dry_run_previews_without_writing() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attest(&db, entity, "Q1").await;

        let cache = FetchCache::new(&db.conn);
        let report = resolver(&db, true)
            .await
            .resolve(
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
            report.preview.first().map(|preview| preview.external_id.as_str()),
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
        let origin = row.origin_observation_id.unwrap();
        KnowledgeGraph2::new(&db)
            .proc(&[GraphMutation::VetoIdentity {
                subject: EntityOriginId(ObservationId(origin)),
                key: IdentityKey {
                    scheme: "fake".into(),
                    external_id: "Q2".into(),
                },
                cause: Cause::Repair,
                why: "test veto".into(),
            }])
            .await
            .unwrap();

        let cache = FetchCache::new(&db.conn);
        let report = resolver(&db, false)
            .await
            .resolve(
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
}
