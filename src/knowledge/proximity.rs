//! Transient proximity candidates: entities the graph currently holds under an identity.
//!
//! There is no batch table and no automatic pass. A scan is a query: sharing an identity is a
//! question, not a persisted fact. One Wikidata item can name an album and cover its songs, so a
//! pair here is a candidate for review, never a verdict.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

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
    pub counterpart_id: i32,
    pub counterpart_name: String,
    /// Every identity the two entities share, in scan order.
    pub identities: Vec<IdentityKey>,
    /// The scorer's pair score; a reading aid, not a decision.
    pub score: f32,
}

/// Return every live pair that shares an identity, with all shared identities, best-scored first.
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
        let Some(holder_id) = ids.first().copied() else {
            continue;
        };
        let holder = &entities[&holder_id];
        for counterpart_id in ids.iter().skip(1).copied() {
            let pair = (holder_id.min(counterpart_id), holder_id.max(counterpart_id));
            let counterpart = &entities[&counterpart_id];
            let candidate = pairs.entry(pair).or_insert_with(|| ProximityCandidate {
                holder_id,
                holder_name: holder.name.clone(),
                counterpart_id,
                counterpart_name: counterpart.name.clone(),
                identities: Vec::new(),
                score: scorer.pair(counterpart, holder),
            });
            candidate.identities.push(IdentityKey {
                scheme: scheme.clone(),
                external_id: external_id.clone(),
            });
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
    }
}
