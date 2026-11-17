//! Apply the recorded decisions to the v2 graph.
//!
//! Only durable repairs are replayed: `repair` merges, newest verdict per pair first, then
//! `repair` vetoes. A merge is logged as `replayed`; a veto is applied through the same
//! `VetoIdentity` mutation as an operator would use. `distinct` is consultative and needs no write.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::knowledge2::graph::{KnowledgeGraph2, Refusal};
use crate::knowledge2::identity::IdentityKey;
use crate::knowledge2::ids::{EntityOriginId, ObservationId};
use crate::knowledge2::mutation::{Cause, GraphMutation};
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
pub async fn replay(db: &DbHandle) -> anyhow::Result<ReplayReport> {
    let graph = KnowledgeGraph2::new(db);
    let mut report = ReplayReport::default();

    // Latest by id wins for a pair, and chains still apply oldest first.
    let mut latest: BTreeMap<(i32, i32), model::knowledge_verdicts::Model> = BTreeMap::new();
    for verdict in merges(db).await? {
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
                cause: Cause::Replayed,
                why: verdict.note.unwrap_or_default(),
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
    for verdict in vetoes(db).await? {
        let (Some(subject), Some(scheme), Some(external)) = (
            verdict.subject_origin,
            verdict.identity_scheme,
            verdict.identity_external_id,
        ) else {
            report.settled += 1;
            continue;
        };
        let Some(entity) = live_entity(db, subject).await? else {
            report.gone += 1;
            continue;
        };
        let key = IdentityKey {
            scheme,
            external_id: external,
        };
        let withdrawn = count_identities(db, entity, &key).await?;

        let outcome = graph
            .proc(&[GraphMutation::VetoIdentity {
                subject: EntityOriginId(ObservationId(subject)),
                key,
                cause: Cause::Repair,
                why: verdict.note.unwrap_or_default(),
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

async fn merges(db: &DbHandle) -> anyhow::Result<Vec<model::knowledge_verdicts::Model>> {
    Ok(model::knowledge_verdicts::Entity::find()
        .filter(
            model::knowledge_verdicts::Column::Cause
                .eq(model::knowledge_verdicts::Cause::Repair.as_str()),
        )
        .filter(
            model::knowledge_verdicts::Column::Kind
                .eq(model::knowledge_verdicts::Kind::Merge.as_str()),
        )
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(&db.conn)
        .await?)
}

async fn vetoes(db: &DbHandle) -> anyhow::Result<Vec<model::knowledge_verdicts::Model>> {
    Ok(model::knowledge_verdicts::Entity::find()
        .filter(
            model::knowledge_verdicts::Column::Cause
                .eq(model::knowledge_verdicts::Cause::Repair.as_str()),
        )
        .filter(
            model::knowledge_verdicts::Column::Kind
                .eq(model::knowledge_verdicts::Kind::VetoIdentity.as_str()),
        )
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(&db.conn)
        .await?)
}

fn pair_key(a: i32, b: i32) -> (i32, i32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Follow the tombstone chain from an origin to the live entity id.
async fn live_entity(db: &DbHandle, origin: i32) -> anyhow::Result<Option<i32>> {
    let Some(row) = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::OriginObservationId.eq(origin))
        .one(&db.conn)
        .await?
    else {
        return Ok(None);
    };

    let mut current = row.id;
    for _ in 0..8 {
        let Some(row) = model::knowledge_entity::Entity::find_by_id(current)
            .one(&db.conn)
            .await?
        else {
            return Ok(None);
        };
        match row.merged_into_id {
            Some(next) => current = next,
            None => return Ok(Some(current)),
        }
    }
    anyhow::bail!("merged_into_id chain from knowledge entity {} exceeds 8 hops", row.id)
}

async fn count_identities(
    db: &DbHandle,
    entity: i32,
    key: &IdentityKey,
) -> anyhow::Result<usize> {
    Ok(model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity))
        .filter(model::knowledge_identity::Column::Scheme.eq(&key.scheme))
        .filter(model::knowledge_identity::Column::ExternalId.eq(&key.external_id))
        .count(&db.conn)
        .await? as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

    use crate::knowledge2::graph::KnowledgeGraph2;
    use crate::knowledge2::identity::{Identity, Provider};
    use crate::knowledge2::mutation::GraphMutation;
    use crate::knowledge2::ids::{EntityOriginId, ObservationId};
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
    async fn a_repair_merge_is_replayed_and_logged_as_replayed() {
        db_or_skip!(db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Winner").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Loser").await;
        let winner_origin = fixtures::origin(&db, winner).await;
        let loser_origin = fixtures::origin(&db, loser).await;
        insert_merge(&db, winner_origin, loser_origin).await;

        let report = replay(&db).await.unwrap();

        assert_eq!(report.applied, 1, "{report:?}");
        let loser = model::knowledge_entity::Entity::find_by_id(loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loser.merged_into_id, Some(winner));
        let replayed = model::knowledge_verdicts::Entity::find()
            .filter(
                model::knowledge_verdicts::Column::Cause
                    .eq(model::knowledge_verdicts::Cause::Replayed.as_str()),
            )
            .filter(
                model::knowledge_verdicts::Column::Kind
                    .eq(model::knowledge_verdicts::Kind::Merge.as_str()),
            )
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(replayed, 1);
    }

    #[tokio::test]
    async fn a_repair_veto_withdraws_and_settles_on_repeat() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Vetoed").await;
        let origin = fixtures::origin(&db, entity).await;
        KnowledgeGraph2::new(&db)
            .proc(&[GraphMutation::Identify {
                subject: EntityOriginId(ObservationId(origin)),
                identity: identity("Q1"),
            }])
            .await
            .unwrap();
        insert_veto(&db, origin, "Q1").await;

        let report = replay(&db).await.unwrap();

        assert_eq!(report.withdrawn, 1, "{report:?}");
        let held = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity))
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(held, 0);

        let again = replay(&db).await.unwrap();
        assert_eq!(again.withdrawn, 0, "{again:?}");
        assert_eq!(again.settled, 1, "{again:?}");
    }
}
