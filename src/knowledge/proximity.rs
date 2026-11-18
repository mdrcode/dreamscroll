//! Transient proximity candidates: entities the graph currently holds under an identity.
//!
//! There is no batch table and no automatic pass. A scan is a query: sharing an identity is a
//! question, not a persisted fact. One Wikidata item can name an album and cover its songs, so a
//! pair here is a candidate for review, never a verdict. A standing `distinct` verdict marks the
//! pair; it does not remove it from the scan.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::database::DbHandle;
use crate::knowledge::heuristic::HeuristicScorer;
use crate::knowledge::identity::IdentityKey;
use crate::model;

/// One pair that shares at least one identity in the current graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ProximityCandidate {
    /// The more substantiated side, used as the reading order.
    pub holder_id: i32,
    pub holder_name: String,
    pub holder_type: String,
    pub counterpart_id: i32,
    pub counterpart_name: String,
    pub counterpart_type: String,
    /// Every identity the two entities share, in scan order.
    pub identities: Vec<IdentityKey>,
    /// A standing `distinct` verdict names this pair.
    pub distinct: bool,
    /// The scorer's pair score; a reading aid, not a decision.
    pub score: f32,
}

/// Return every live pair that shares an identity, with all shared identities, best-scored first.
/// A standing `distinct` verdict is marked on the pair.
pub async fn scan(db: &DbHandle) -> anyhow::Result<Vec<ProximityCandidate>> {
    let entities = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
        .all(&db.conn)
        .await?;
    let entities: HashMap<i32, model::knowledge_entity::Model> = entities
        .into_iter()
        .map(|entity| (entity.id, entity))
        .collect();

    let identities = model::knowledge_identity::Entity::find()
        .all(&db.conn)
        .await?;
    let distinct_pairs: HashSet<(i32, i32)> = model::knowledge_verdicts::Entity::find()
        .filter(
            model::knowledge_verdicts::Column::Kind
                .eq(model::knowledge_verdicts::Kind::Distinct.as_str()),
        )
        .all(&db.conn)
        .await?
        .into_iter()
        .filter_map(|verdict| {
            let a = verdict.winner_origin?;
            let b = verdict.loser_origin?;
            Some((a.min(b), a.max(b)))
        })
        .collect();
    let mut holders: BTreeMap<(String, String), Vec<i32>> = BTreeMap::new();
    for identity in identities {
        if entities.contains_key(&identity.knowledge_entity_id) {
            holders
                .entry((identity.scheme, identity.external_id))
                .or_default()
                .push(identity.knowledge_entity_id);
        }
    }

    let scorer = HeuristicScorer;
    let mut pairs: HashMap<(i32, i32), ProximityCandidate> = HashMap::new();
    for ((scheme, external_id), mut ids) in holders {
        ids.sort_by(|a, b| {
            let a = &entities[a];
            let b = &entities[b];
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        ids.dedup();
        for (i, &holder_id) in ids.iter().enumerate() {
            for &counterpart_id in &ids[i + 1..] {
                let pair = (holder_id.min(counterpart_id), holder_id.max(counterpart_id));
                let holder = &entities[&holder_id];
                let counterpart = &entities[&counterpart_id];
                let candidate = pairs.entry(pair).or_insert_with(|| ProximityCandidate {
                    holder_id,
                    holder_name: holder.name.clone(),
                    holder_type: holder.entity_type.clone(),
                    counterpart_id,
                    counterpart_name: counterpart.name.clone(),
                    counterpart_type: counterpart.entity_type.clone(),
                    identities: Vec::new(),
                    distinct: distinct_pairs.contains(&(
                        holder
                            .origin_observation_id
                            .min(counterpart.origin_observation_id),
                        holder
                            .origin_observation_id
                            .max(counterpart.origin_observation_id),
                    )),
                    score: scorer.pair(counterpart, holder),
                });
                candidate.identities.push(IdentityKey {
                    scheme: scheme.clone(),
                    external_id: external_id.clone(),
                });
            }
        }
    }

    let mut candidates: Vec<_> = pairs.into_values().collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.holder_id.cmp(&b.holder_id))
            .then_with(|| a.counterpart_id.cmp(&b.counterpart_id))
    });
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn scan_returns_one_candidate_with_all_shared_identities() {
        db_or_skip!(db);
        let left = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let right = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::identity(&db, left, "wikidata", "Q1", "test", None, Some(0.9)).await;
        fixtures::identity(&db, right, "wikidata", "Q1", "test", None, Some(0.8)).await;
        fixtures::identity(&db, left, "wikidata", "Q2", "test", None, Some(0.9)).await;
        fixtures::identity(&db, right, "wikidata", "Q2", "test", None, Some(0.8)).await;

        let candidates = scan(&db).await.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].identities,
            vec![
                IdentityKey {
                    scheme: "wikidata".into(),
                    external_id: "Q1".into(),
                },
                IdentityKey {
                    scheme: "wikidata".into(),
                    external_id: "Q2".into(),
                },
            ]
        );
        assert_eq!(
            (candidates[0].holder_id, candidates[0].counterpart_id),
            (left, right)
        );
        assert!(!candidates[0].distinct);
    }

    #[tokio::test]
    async fn scan_pairs_every_holder_with_every_other_holder() {
        db_or_skip!(db);
        let high = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let middle = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let low = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        for (entity, confidence) in [(high, 0.9), (middle, 0.6), (low, 0.3)] {
            model::knowledge_entity::Entity::update_many()
                .col_expr(
                    model::knowledge_entity::Column::Confidence,
                    sea_orm::sea_query::Expr::value(Some(confidence)),
                )
                .filter(model::knowledge_entity::Column::Id.eq(entity))
                .exec(&db.conn)
                .await
                .unwrap();
        }
        for entity in [high, middle, low] {
            fixtures::identity(&db, entity, "wikidata", "Q1", "test", None, Some(0.9)).await;
        }

        let candidates = scan(&db).await.unwrap();
        assert_eq!(candidates.len(), 3);
        let pairs: HashSet<(i32, i32)> = candidates
            .iter()
            .map(|candidate| (candidate.holder_id, candidate.counterpart_id))
            .collect();
        assert_eq!(
            pairs,
            HashSet::from([(high, middle), (high, low), (middle, low)])
        );
        for candidate in &candidates {
            assert_eq!(
                candidate.identities,
                vec![IdentityKey {
                    scheme: "wikidata".into(),
                    external_id: "Q1".into(),
                }]
            );
            assert!(!candidate.distinct);
        }
    }

    #[tokio::test]
    async fn scan_marks_a_standing_distinct() {
        db_or_skip!(db);
        let left = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let right = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::identity(&db, left, "wikidata", "Q1", "test", None, Some(0.9)).await;
        fixtures::identity(&db, right, "wikidata", "Q1", "test", None, Some(0.8)).await;

        let graph = crate::knowledge::KnowledgeGraph::new(&db);
        let a = crate::knowledge::EntityOriginId(crate::knowledge::ObservationId(
            fixtures::origin(&db, left).await,
        ));
        let b = crate::knowledge::EntityOriginId(crate::knowledge::ObservationId(
            fixtures::origin(&db, right).await,
        ));
        let outcome = graph
            .proc(&[crate::knowledge::GraphMutation::MarkDistinct {
                a,
                b,
                cause: crate::knowledge::Cause::Repair,
                why: "test".into(),
            }])
            .await
            .unwrap();
        assert!(outcome.refused.is_none());

        let candidates = scan(&db).await.unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].distinct);
    }
}
