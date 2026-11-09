//! Durable human decisions: which two things are one, kept across rebuilds.
//!
//! A merge is entity → entity in the present tense. A rebuild renumbers entities, so the record is
//! keyed by each side's `origin_observation_id` — observations are never deleted or renumbered, and
//! `link` builds entities in a deterministic order, so the origin is a stable name for a thing.
//!
//! Replay asks the graph what each origin names now and folds the pairs that still map. Anything
//! that does not is reported and dropped: erosion is the price of the key, and a silent wrong merge
//! is not.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::model;

use crate::knowledge::evidence::ClaimBasis;
use super::merge::{Cause, Outcome};
use super::KnowledgeGraph;

/// What one replay did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Pairs folded onto the current graph.
    pub applied: usize,
    /// Pairs the graph already had as one entity.
    pub already: usize,
    /// Verdicts whose sides no longer name a live entity each.
    pub gone: usize,
    /// Pairs the shared-illumination rule refused.
    pub refused: usize,
    /// Live claims withdrawn because an entity was vetoed out of an identity.
    pub withdrawn: usize,
}

/// Which statement a verdict row makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Two origins are one thing.
    Merge,
    /// An entity is not an identity. The item stays valid and claimable by everything else.
    Veto,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Veto => "veto",
        }
    }
}

/// The generation the graph is being built in. One is implicit on a database that never reset.
pub(crate) async fn current_generation<C: ConnectionTrait>(conn: &C) -> anyhow::Result<i32> {
    Ok(model::knowledge_generations::Entity::find()
        .order_by_desc(model::knowledge_generations::Column::Id)
        .one(conn)
        .await?
        .map_or(1, |row| row.id))
}

/// The unordered pair two origins name, as a key.
pub(crate) fn pair_key(a: i32, b: i32) -> (i32, i32) {
    (a.min(b), a.max(b))
}

/// Record a merge. Idempotent per cause, generation and direction, so replaying a repair does not
/// grow the log and an automatic fold is only ever noted once in the generation it happened in.
pub(super) async fn record<C: ConnectionTrait>(
    conn: &C,
    cause: Cause,
    winner: &model::knowledge_entity::Model,
    loser: &model::knowledge_entity::Model,
) -> anyhow::Result<bool> {
    let (Some(winner_origin), Some(loser_origin)) =
        (winner.origin_observation_id, loser.origin_observation_id)
    else {
        // A row without an origin cannot be named after a rebuild. Tests and hand-seeded rows can
        // lack one; a merge they take part in simply does not enter the log.
        return Ok(false);
    };
    let generation = current_generation(conn).await?;
    let existing = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(cause.as_str()))
        .filter(model::knowledge_verdicts::Column::Generation.eq(generation))
        .filter(model::knowledge_verdicts::Column::WinnerOrigin.eq(winner_origin))
        .filter(model::knowledge_verdicts::Column::LoserOrigin.eq(loser_origin))
        .one(conn)
        .await?;
    if existing.is_some() {
        return Ok(false);
    }
    model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
        generation: Set(generation),
        cause: Set(cause.as_str().to_string()),
        kind: Set(Kind::Merge.as_str().to_string()),
        winner_origin: Set(Some(winner_origin)),
        loser_origin: Set(Some(loser_origin)),
        winner_label: Set(Some(winner.name.clone())),
        loser_label: Set(Some(loser.name.clone())),
        ..Default::default()
    })
    .exec(conn)
    .await?;
    Ok(true)
}

/// Apply the recorded verdicts to the current graph, newest verdict per pair first.
pub async fn replay(graph: &KnowledgeGraph) -> anyhow::Result<ReplayReport> {
    // Only a person's decisions are replayed. The model's folds and earlier replays are history.
    let verdicts = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Merge.as_str()))
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(graph.conn())
        .await?;

    // Latest by id wins for a pair, and chains still apply oldest first.
    let mut latest: BTreeMap<(i32, i32), model::knowledge_verdicts::Model> = BTreeMap::new();
    for verdict in verdicts {
        let (Some(winner), Some(loser)) = (verdict.winner_origin, verdict.loser_origin) else {
            continue;
        };
        latest.insert(pair_key(winner, loser), verdict);
    }
    let mut ordered: Vec<model::knowledge_verdicts::Model> = latest.into_values().collect();
    ordered.sort_by_key(|verdict| verdict.id);

    let mut report = ReplayReport::default();
    for verdict in ordered {
        // Tombstones count: a verdict applied in this generation leaves the loser's row behind,
        // pointing at the winner. "Already" and "gone" must not be confused.
        let (Some(winner_origin), Some(loser_origin)) =
            (verdict.winner_origin, verdict.loser_origin)
        else {
            continue;
        };
        let (Some(winner), Some(loser)) = (
            entity_with_origin(graph, winner_origin).await?,
            entity_with_origin(graph, loser_origin).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        // Follow the tombstones: the model may have folded the person's winner into something else,
        // and the decision still means "these two are one" about wherever they are now.
        let (Some(winner), Some(loser)) = (
            canonical(graph, winner.id).await?,
            canonical(graph, loser.id).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        if winner == loser {
            report.already += 1;
            continue;
        }
        match graph.merge(loser, winner, Cause::Replayed).await? {
            Outcome::Merged => report.applied += 1,
            Outcome::AlreadyMerged => report.already += 1,
            Outcome::RefusedSharedIllumination => report.refused += 1,
        }
    }

    // Vetoes last. A merge decides *which* entity a verdict is about, so a veto is applied to
    // wherever its subject ended up, not to the entity the editor happened to be looking at.
    let vetoes = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(graph.conn())
        .await?;
    for verdict in vetoes {
        match withdraw(graph, &verdict).await? {
            Some(retracted) => report.withdrawn += retracted,
            None => report.gone += 1,
        }
    }
    Ok(report)
}

/// Record that an entity is *not* an identity.
///
/// Idempotent per subject and identity, and deliberately not per generation: a fold recurs every
/// build, but a veto is a standing decision about the graph.
pub async fn veto(
    graph: &KnowledgeGraph,
    entity_id: i32,
    scheme: &str,
    external_id: &str,
    note: Option<&str>,
) -> anyhow::Result<bool> {
    let Some(entity) = model::knowledge_entity::Entity::find_by_id(entity_id)
        .one(graph.conn())
        .await?
    else {
        anyhow::bail!("no knowledge entity {entity_id}");
    };
    let Some(subject) = entity.origin_observation_id else {
        anyhow::bail!("entity {entity_id} has no origin observation, so no verdict can name it");
    };
    let generation = current_generation(graph.conn()).await?;
    let existing = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .filter(model::knowledge_verdicts::Column::SubjectOrigin.eq(subject))
        .filter(model::knowledge_verdicts::Column::IdentityScheme.eq(scheme))
        .filter(model::knowledge_verdicts::Column::IdentityExternalId.eq(external_id))
        .one(graph.conn())
        .await?;
    if existing.is_some() {
        return Ok(false);
    }
    model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
        generation: Set(generation),
        cause: Set(Cause::Repair.as_str().to_string()),
        kind: Set(Kind::Veto.as_str().to_string()),
        subject_origin: Set(Some(subject)),
        identity_scheme: Set(Some(scheme.to_string())),
        identity_external_id: Set(Some(external_id.to_string())),
        note: Set(note.map(str::to_string)),
        ..Default::default()
    })
    .exec(graph.conn())
    .await?;
    Ok(true)
}

/// Lift a veto. Nothing is un-retracted: lifting stops suppressing a decision, and the next resolve
/// re-derives the claim as a new row, because claims are append-only.
pub async fn unveto(graph: &KnowledgeGraph, verdict_id: i32) -> anyhow::Result<bool> {
    let deleted = model::knowledge_verdicts::Entity::delete_many()
        .filter(model::knowledge_verdicts::Column::Id.eq(verdict_id))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .exec(graph.conn())
        .await?
        .rows_affected;
    Ok(deleted > 0)
}

/// The `(entity, scheme, external_id)` triples a live veto forbids.
///
/// Admission asks this before a claim is written: a claim that is never written cannot leave a window
/// where the graph asserts something an editor has rejected, and does not need a replay to undo.
pub async fn vetoed(
    graph: &KnowledgeGraph,
) -> anyhow::Result<std::collections::HashSet<(i32, String, String)>> {
    let mut out = std::collections::HashSet::new();
    let vetoes = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .all(graph.conn())
        .await?;
    for verdict in vetoes {
        let (Some(subject), Some(scheme), Some(external)) = (
            verdict.subject_origin,
            verdict.identity_scheme.clone(),
            verdict.identity_external_id.clone(),
        ) else {
            continue;
        };
        let Some(row) = entity_with_origin(graph, subject).await? else {
            continue;
        };
        let Some(entity_id) = canonical(graph, row.id).await? else {
            continue;
        };
        out.insert((entity_id, scheme, external));
    }
    Ok(out)
}

/// Retract the live authority claims holding a vetoed identity for its entity.
///
/// Basis `text` only: a `handle` claim is what a capture showed, not a cross-reference an editor is
/// disputing. `None` when the subject origin no longer names a live entity.
async fn withdraw(
    graph: &KnowledgeGraph,
    verdict: &model::knowledge_verdicts::Model,
) -> anyhow::Result<Option<usize>> {
    let (Some(subject), Some(scheme), Some(external)) = (
        verdict.subject_origin,
        verdict.identity_scheme.as_deref(),
        verdict.identity_external_id.as_deref(),
    ) else {
        return Ok(Some(0));
    };
    let Some(row) = entity_with_origin(graph, subject).await? else {
        return Ok(None);
    };
    let Some(entity_id) = canonical(graph, row.id).await? else {
        return Ok(None);
    };
    let Some(identity) = model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::Scheme.eq(scheme))
        .filter(model::knowledge_identity::Column::ExternalId.eq(external))
        .one(graph.conn())
        .await?
    else {
        return Ok(Some(0));
    };
    let retracted = model::knowledge_claims::Entity::update_many()
        .col_expr(
            model::knowledge_claims::Column::RetractedAt,
            sea_orm::sea_query::Expr::current_timestamp(),
        )
        .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity_id))
        .filter(model::knowledge_claims::Column::KnowledgeIdentityId.eq(identity.id))
        .filter(model::knowledge_claims::Column::Basis.eq(ClaimBasis::Text.as_str()))
        .filter(model::knowledge_claims::Column::RetractedAt.is_null())
        .exec(graph.conn())
        .await?
        .rows_affected as usize;
    Ok(Some(retracted))
}

/// The entity this origin names now, live or tombstoned.
async fn entity_with_origin(
    graph: &KnowledgeGraph,
    origin: i32,
) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
    Ok(model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::OriginObservationId.eq(origin))
        .one(graph.conn())
        .await?)
}

/// Follow tombstones to the live entity a row ends at. Bounded: a cycle is not expressible in the
/// schema, but the reader must not spin if one somehow appears.
async fn canonical(graph: &KnowledgeGraph, id: i32) -> anyhow::Result<Option<i32>> {
    let mut current = id;
    for _ in 0..8 {
        let Some(row) = model::knowledge_entity::Entity::find_by_id(current)
            .one(graph.conn())
            .await?
        else {
            return Ok(None);
        };
        match row.merged_into_id {
            Some(next) => current = next,
            None => return Ok(Some(current)),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use crate::test_support::{db_or_skip, fixtures};

    use super::*;

    /// Seed an entity claiming one identity, as `resolve` would have written it.
    async fn seed_claim(
        db: &crate::database::DbHandle,
        name: &str,
        external_id: &str,
    ) -> i32 {
        let entity_id = fixtures::knowledge_entity_with_member(db, name).await;
        let identity = match model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::Scheme.eq("wikidata"))
            .filter(model::knowledge_identity::Column::ExternalId.eq(external_id))
            .one(&db.conn)
            .await
            .unwrap()
        {
            Some(identity) => identity,
            None => model::knowledge_identity::Entity::insert(
                model::knowledge_identity::ActiveModel {
                    scheme: Set("wikidata".to_string()),
                    external_id: Set(external_id.to_string()),
                    asserted_by: Set("wikidata".to_string()),
                    classes: Set(Vec::new()),
                    ..Default::default()
                },
            )
            .exec_with_returning(&db.conn)
            .await
            .unwrap(),
        };
        model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
            knowledge_entity_id: Set(entity_id),
            knowledge_identity_id: Set(identity.id),
            basis: Set(ClaimBasis::Text.as_str().to_string()),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
        entity_id
    }

    async fn live_claims(db: &crate::database::DbHandle, entity_id: i32) -> usize {
        model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity_id))
            .filter(model::knowledge_claims::Column::RetractedAt.is_null())
            .all(&db.conn)
            .await
            .unwrap()
            .len()
    }

    /// A veto withdraws the one pairing it names — not the identity, and not the entity.
    #[tokio::test]
    async fn a_veto_withdraws_only_the_pairing_it_names() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let vetoed_entity = seed_claim(&db, "Q", "Q131472411").await;
        // The same item, legitimately claimed by a different entity: the veto must not reach it.
        let other = seed_claim(&db, "Q (Star Trek)", "Q131472411").await;

        assert!(
            veto(&store, vetoed_entity, "wikidata", "Q131472411", Some("a short fiction"))
                .await
                .unwrap()
        );
        assert!(
            !veto(&store, vetoed_entity, "wikidata", "Q131472411", None)
                .await
                .unwrap(),
            "recording the same veto twice is a no-op"
        );

        let report = replay(&store).await.unwrap();
        assert_eq!(report.withdrawn, 1, "{report:?}");
        assert_eq!(live_claims(&db, vetoed_entity).await, 0);
        assert_eq!(live_claims(&db, other).await, 1, "the pair, not the identity");
    }

    /// The claim is re-derived every build, so the veto has to outlive the rows it withdrew.
    #[tokio::test]
    async fn a_veto_survives_a_rebuild_and_withdraws_the_new_claim() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = seed_claim(&db, "Q", "Q131472411").await;
        let subject = origin(&db, entity_id).await;
        veto(&store, entity_id, "wikidata", "Q131472411", None)
            .await
            .unwrap();
        assert_eq!(replay(&store).await.unwrap().withdrawn, 1);

        // A rebuild: entities, identities and claims are recreated and the parser claims again.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn = seed_claim(&db, "Q", "Q131472411").await;
        set_origin(&db, reborn, subject).await;

        let report = replay(&store).await.unwrap();
        assert_eq!(report.withdrawn, 1, "against the reborn claim: {report:?}");
        assert_eq!(live_claims(&db, reborn).await, 0);
        assert_eq!(vetoed(&store).await.unwrap().len(), 1, "admission still knows it");
    }

    /// Lifting is deletion. Nothing is un-retracted: the claim returns by being re-derived.
    #[tokio::test]
    async fn lifting_a_veto_stops_suppressing_and_restores_nothing() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = seed_claim(&db, "Los sorias", "Q122707294").await;
        veto(&store, entity_id, "wikidata", "Q122707294", Some("the edition, not the work"))
            .await
            .unwrap();
        replay(&store).await.unwrap();
        assert_eq!(live_claims(&db, entity_id).await, 0);

        let id = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .id;
        assert!(unveto(&store, id).await.unwrap());
        assert!(!unveto(&store, id).await.unwrap(), "already gone");
        assert!(vetoed(&store).await.unwrap().is_empty());
        assert_eq!(
            live_claims(&db, entity_id).await,
            0,
            "lifting is passive; the next resolve re-derives"
        );
        assert_eq!(replay(&store).await.unwrap().withdrawn, 0);
    }

    /// The automatic pass logs its fold with its own cause, and replay leaves it alone: the model's
    /// opinion is re-derived every build, never restored.
    #[tokio::test]
    async fn an_automatic_merge_is_logged_but_not_replayed() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;

        store
            .merge(loser, winner, Cause::ProximityAuto)
            .await
            .unwrap();

        let logged = model::knowledge_verdicts::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].cause, "proximity_auto");

        let report = replay(&store).await.expect("replay");
        assert_eq!(report, ReplayReport::default(), "nothing to replay");
    }

    /// Replay folds what a previous build had merged, by origin.
    #[tokio::test]
    async fn replay_folds_the_pair_again() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let winner_origin = origin(&db, winner).await;
        let loser_origin = origin(&db, loser).await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        // A rebuild: the graph is emptied, the observations and the verdict survive it.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        set_origin(&db, reborn_winner, winner_origin).await;
        set_origin(&db, reborn_loser, loser_origin).await;

        let report = replay(&store).await.expect("replay");

        assert_eq!(report.applied, 1, "{report:?}");
        let tombstone = model::knowledge_entity::Entity::find_by_id(reborn_loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(tombstone.merged_into_id.is_some(), "the recorded side folded");
    }

    /// The model may fold a person's winner into something else. The decision still means "these two
    /// are one", so the replay follows the tombstone and folds into wherever the winner ended up.
    #[tokio::test]
    async fn replay_follows_a_winner_the_model_folded_away() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let winner_origin = origin(&db, winner).await;
        let loser_origin = origin(&db, loser).await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        // A rebuild gives the pair back, then the model folds the person's winner into a third
        // entity: the winner is now a tombstone, and the loser is not.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        set_origin(&db, reborn_winner, winner_origin).await;
        set_origin(&db, reborn_loser, loser_origin).await;
        let heir = fixtures::knowledge_entity_with_member(&db, "Train Dreams (film)").await;
        store
            .merge(reborn_winner, heir, Cause::ProximityAuto)
            .await
            .unwrap();

        let report = replay(&store).await.expect("replay");

        assert_eq!(report.applied, 1, "{report:?}");
        assert_eq!(report.gone, 0, "{report:?}");
        let tombstone = model::knowledge_entity::Entity::find_by_id(reborn_loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            tombstone.merged_into_id,
            Some(heir),
            "the decision retargets through the winner's tombstone"
        );
    }

    /// Applied once, reported as already applied on every later run — a tombstone is not erosion.
    #[tokio::test]
    async fn replay_is_idempotent() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        let report = replay(&store).await.expect("replay");

        assert_eq!(report.already, 1, "{report:?}");
        assert_eq!(report.applied, 0, "{report:?}");
        assert_eq!(report.gone, 0, "{report:?}");
    }

    /// A verdict whose sides are not both live is reported, not guessed.
    #[tokio::test]
    async fn replay_reports_sides_that_are_gone() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let only = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let origin = origin(&db, only).await;
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            winner_origin: Set(Some(origin)),
            loser_origin: Set(Some(origin + 1000)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        let report = replay(&store).await.expect("replay");

        assert_eq!(report.gone, 1, "{report:?}");
        assert_eq!(report.applied, 0, "{report:?}");
    }

    async fn origin(db: &crate::database::DbHandle, id: i32) -> i32 {
        model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .origin_observation_id
            .expect("fixtures set the origin")
    }

    async fn set_origin(db: &crate::database::DbHandle, id: i32, origin: i32) {
        model::knowledge_entity::Entity::update_many()
            .col_expr(
                model::knowledge_entity::Column::OriginObservationId,
                sea_orm::sea_query::Expr::value(Some(origin)),
            )
            .filter(model::knowledge_entity::Column::Id.eq(id))
            .exec(&db.conn)
            .await
            .unwrap();
    }

}
