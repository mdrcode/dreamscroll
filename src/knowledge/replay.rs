//! Apply the recorded decisions to the v2 graph.
//!
//! Only durable repairs are replayed: `repair` merges, newest verdict per pair first, then
//! `repair` vetoes. Replay applies the decision without appending another verdict row; the log is
//! decisions, not history. `distinct` is consultative and needs no write.

use std::collections::BTreeMap;

use crate::knowledge::graph::{KnowledgeGraph, Refusal};
use crate::knowledge::identity::IdentityKey;
use crate::knowledge::ids::{EntityOriginId, ObservationId};
use crate::knowledge::mutation::{Cause, GraphMutation};
use crate::model;

/// What one replay did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Pairs folded onto the current graph.
    pub applied: usize,
    /// Pairs the graph already had as one entity.
    pub already: usize,
    /// Verdicts whose sides no longer name a live entity each.
    pub gone: usize,
    /// Pairs a rule refused: shared illumination, or a standing `distinct`.
    pub refused: usize,
    /// Identity annotations deleted because an entity was vetoed out of an identity.
    pub withdrawn: usize,
    /// Vetoes already in force: nothing to delete.
    pub settled: usize,
}

/// Apply the recorded verdicts to the current graph.
pub async fn replay(graph: &KnowledgeGraph) -> anyhow::Result<ReplayReport> {
    let selector = graph.selector();
    let verdicts = selector.verdicts();
    let mut report = ReplayReport::default();

    // Latest by id wins for a pair, and chains still apply oldest first.
    let mut latest: BTreeMap<(i32, i32), model::knowledge_verdicts::Model> = BTreeMap::new();
    for verdict in verdicts.merges().await? {
        let (Some(winner), Some(loser)) = (verdict.winner_origin, verdict.loser_origin) else {
            continue;
        };
        latest.insert(pair_key(winner, loser), verdict);
    }
    let mut ordered: Vec<model::knowledge_verdicts::Model> = latest.into_values().collect();
    ordered.sort_by_key(|verdict| verdict.id);

    for verdict in ordered {
        let (Some(winner), Some(loser)) = (verdict.winner_origin, verdict.loser_origin) else {
            continue;
        };
        let outcome = graph
            .proc(&[GraphMutation::MergeEntity {
                loser: EntityOriginId(ObservationId(loser)),
                winner: EntityOriginId(ObservationId(winner)),
                cause: Cause::Repair,
                why: verdict.note.unwrap_or_default(),
                record_verdict: false,
            }])
            .await?;

        if let Some(refusal) = outcome.refused {
            match refusal {
                Refusal::UnknownOrigin(_) | Refusal::AmbiguousOrigin(_) => report.gone += 1,
                _ => report.refused += 1,
            }
            continue;
        }
        if outcome.applied > 0 {
            report.applied += 1;
        } else if outcome.unchanged > 0 {
            report.already += 1;
        }
    }

    // Vetoes last: a merge decides which entity a veto is about, so it lands on the survivor.
    for verdict in verdicts.vetoes().await? {
        let (Some(subject), Some(scheme), Some(external)) = (
            verdict.subject_origin,
            verdict.identity_scheme,
            verdict.identity_external_id,
        ) else {
            report.settled += 1;
            continue;
        };
        let Some(entity) = selector
            .live_entity(EntityOriginId(ObservationId(subject)))
            .await?
        else {
            report.gone += 1;
            continue;
        };
        let key = IdentityKey {
            scheme,
            external_id: external,
        };
        let withdrawn = selector.identity_count(entity, &key).await?;

        let outcome = graph
            .proc(&[GraphMutation::VetoIdentity {
                subject: EntityOriginId(ObservationId(subject)),
                key,
                cause: Cause::Repair,
                why: verdict.note.unwrap_or_default(),
                record_verdict: false,
            }])
            .await?;

        if let Some(refusal) = outcome.refused {
            match refusal {
                Refusal::UnknownOrigin(_) | Refusal::AmbiguousOrigin(_) => report.gone += 1,
                _ => report.refused += 1,
            }
            continue;
        }
        if withdrawn > 0 {
            report.withdrawn += withdrawn;
        } else {
            report.settled += 1;
        }
    }

    Ok(report)
}

fn pair_key(a: i32, b: i32) -> (i32, i32) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

    use crate::database::DbHandle;
    use crate::knowledge::graph::KnowledgeGraph;
    use crate::knowledge::identity::{Identity, Provider};
    use crate::knowledge::mutation::GraphMutation;
    use crate::knowledge::ids::{EntityOriginId, ObservationId};
    use crate::test_support::{db_or_skip, fixtures};

    fn identity(external_id: &str) -> Identity {
        Identity {
            provider: Provider("wikidata".into()),
            scheme: "wikidata".into(),
            external_id: external_id.into(),
            method: Some("test".into()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: None,
        }
    }

    async fn insert_merge(db: &DbHandle, winner: i32, loser: i32) {
        model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            cause: Set(model::knowledge_verdicts::Cause::Repair),
            kind: Set(model::knowledge_verdicts::Kind::Merge),
            winner_origin: Set(Some(winner)),
            loser_origin: Set(Some(loser)),
            note: Set(Some("test merge".into())),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();
    }

    async fn insert_veto(db: &DbHandle, subject: i32, external_id: &str) {
        model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            cause: Set(model::knowledge_verdicts::Cause::Repair),
            kind: Set(model::knowledge_verdicts::Kind::VetoIdentity),
            subject_origin: Set(Some(subject)),
            identity_scheme: Set(Some("wikidata".into())),
            identity_external_id: Set(Some(external_id.into())),
            note: Set(Some("test veto".into())),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_repair_merge_is_replayed_without_appending_a_verdict() {
        db_or_skip!(db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Winner").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Loser").await;
        let winner_origin = fixtures::origin(&db, winner).await;
        let loser_origin = fixtures::origin(&db, loser).await;
        insert_merge(&db, winner_origin, loser_origin).await;

        let report = replay(&KnowledgeGraph::new(&db)).await.unwrap();

        assert_eq!(report.applied, 1, "{report:?}");
        let loser = model::knowledge_entity::Entity::find_by_id(loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loser.merged_into_id, Some(winner));
        let verdicts = model::knowledge_verdicts::Entity::find()
            .filter(
                model::knowledge_verdicts::Column::Kind
                    .eq(model::knowledge_verdicts::Kind::Merge.as_str()),
            )
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(verdicts, 1, "replay must not append a merge decision");
    }

    #[tokio::test]
    async fn a_repair_veto_withdraws_and_settles_on_repeat() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Vetoed").await;
        let origin = fixtures::origin(&db, entity).await;
        let graph = KnowledgeGraph::new(&db);
        graph
            .proc(&[GraphMutation::Identify {
                subject: EntityOriginId(ObservationId(origin)),
                identity: identity("Q1"),
            }])
            .await
            .unwrap();
        insert_veto(&db, origin, "Q1").await;

        let report = replay(&graph).await.unwrap();

        assert_eq!(report.withdrawn, 1, "{report:?}");
        let verdicts = model::knowledge_verdicts::Entity::find()
            .filter(
                model::knowledge_verdicts::Column::Kind
                    .eq(model::knowledge_verdicts::Kind::VetoIdentity.as_str()),
            )
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(verdicts, 1, "replay must not append a veto decision");
        let held = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity))
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(held, 0);

        let again = replay(&graph).await.unwrap();
        assert_eq!(again.withdrawn, 0, "{again:?}");
        assert_eq!(again.settled, 1, "{again:?}");
    }
}
