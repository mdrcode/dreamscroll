use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::knowledge::graph::{KnowledgeGraph, ReplayReport};
use crate::model;
use crate::model::knowledge_verdicts::Kind;

use super::Cause;

use crate::test_support::{db_or_skip, fixtures};

/// Seed an entity annotated with one identity, as `resolve` would have written it.
async fn seed_claim(db: &crate::database::DbHandle, name: &str, external_id: &str) -> i32 {
    let entity_id = fixtures::knowledge_entity_with_member(db, name).await;
    fixtures::identity(
        db,
        entity_id,
        "wikidata",
        external_id,
        "wikidata",
        None,
        None,
    )
    .await;
    entity_id
}

async fn live_claims(db: &crate::database::DbHandle, entity_id: i32) -> usize {
    model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity_id))
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

    let first = store
        .veto(
            vetoed_entity,
            "wikidata",
            "Q131472411",
            Some("a short fiction"),
        )
        .await
        .unwrap();
    assert!(first.recorded);
    assert_eq!(first.withdrawn, 1, "the veto applies when it is recorded");
    let again = store
        .veto(vetoed_entity, "wikidata", "Q131472411", None)
        .await
        .unwrap();
    assert!(!again.recorded, "recording the same veto twice is a no-op");
    assert_eq!(again.withdrawn, 0, "the claim is already retracted");

    let report = store.replay_verdicts().await.unwrap();
    assert_eq!(report.settled, 1, "{report:?}");
    assert_eq!(report.withdrawn, 0, "{report:?}");
    assert_eq!(live_claims(&db, vetoed_entity).await, 0);
    assert_eq!(
        live_claims(&db, other).await,
        1,
        "the pair, not the identity"
    );
}

/// A veto names a pairing, so it retracts the claim whatever route produced it — including an
/// authority that reached it through a handle (P2002).
#[tokio::test]
async fn a_veto_retracts_a_claim_that_arrived_by_handle() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let entity_id = fixtures::knowledge_entity_with_member(&db, "Tsoding").await;
    fixtures::identity(
        &db,
        entity_id,
        "wikidata",
        "Q123737256",
        "wikidata",
        Some("P2002"),
        Some(0.9),
    )
    .await;
    store.update_projection(entity_id).await.unwrap();
    assert_eq!(
        store.get(entity_id).await.unwrap().status,
        crate::model::knowledge_entity::Status::Identified
    );

    let veto = store
        .veto(entity_id, "wikidata", "Q123737256", None)
        .await
        .unwrap();
    assert_eq!(veto.withdrawn, 1, "{veto:?}");

    let entity = store.get(entity_id).await.unwrap();
    assert_eq!(
        entity.status,
        crate::model::knowledge_entity::Status::Provisional,
        "the identity went, so the identification went"
    );
    assert_eq!(entity.confidence, None);

    // A second replay: the decision holds, and says so instead of reporting a zero withdrawal.
    let report = store.replay_verdicts().await.unwrap();
    assert_eq!(report.settled, 1, "{report:?}");
    assert_eq!(report.withdrawn, 0, "{report:?}");
}

/// The claim is re-derived every build, so the veto has to outlive the rows it withdrew.
#[tokio::test]
async fn a_veto_survives_a_rebuild_and_withdraws_the_new_claim() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let entity_id = seed_claim(&db, "Q", "Q131472411").await;
    let subject = fixtures::origin(&db, entity_id).await;
    let veto = store
        .veto(entity_id, "wikidata", "Q131472411", None)
        .await
        .unwrap();
    assert_eq!(veto.withdrawn, 1);

    // A rebuild: entities, identities and claims are recreated and the parser claims again.
    crate::knowledge::snapshot::reset(&db).await.unwrap();
    let reborn = seed_claim(&db, "Q", "Q131472411").await;
    fixtures::set_origin(&db, reborn, subject).await;

    let report = store.replay_verdicts().await.unwrap();
    assert_eq!(report.withdrawn, 1, "against the reborn claim: {report:?}");
    assert_eq!(live_claims(&db, reborn).await, 0);
    assert_eq!(
        store.vetoed().await.unwrap().len(),
        1,
        "admission still knows it"
    );
}

/// Lifting is deletion. Nothing is un-retracted: the claim returns by being re-derived.
#[tokio::test]
async fn lifting_a_veto_stops_suppressing_and_restores_nothing() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let entity_id = seed_claim(&db, "Los sorias", "Q122707294").await;
    store
        .veto(
            entity_id,
            "wikidata",
            "Q122707294",
            Some("the edition, not the work"),
        )
        .await
        .unwrap();
    assert_eq!(live_claims(&db, entity_id).await, 0);

    let id = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::VetoIdentity.as_str()))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
        .id;
    assert!(store.unveto(id).await.unwrap());
    assert!(!store.unveto(id).await.unwrap(), "already gone");
    assert!(store.vetoed().await.unwrap().is_empty());
    assert_eq!(
        live_claims(&db, entity_id).await,
        0,
        "lifting is passive; the next resolve re-derives"
    );
    assert_eq!(store.replay_verdicts().await.unwrap().withdrawn, 0);
}

/// Status and confidence are functions of the live claims. Retracting one without reprojecting
/// leaves a row the API serves as `identified` and `proximity auto-merge` ranks by a score it no
/// longer earns.
#[tokio::test]
async fn a_veto_reprojects_the_entity_it_empties() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let entity_id = fixtures::knowledge_entity_with_member(&db, "Q").await;
    fixtures::identity(
        &db,
        entity_id,
        "wikidata",
        "Q131472411",
        "wikidata",
        Some("exact_label"),
        Some(0.9),
    )
    .await;
    store.update_projection(entity_id).await.unwrap();
    assert_eq!(
        store.get(entity_id).await.unwrap().status,
        crate::model::knowledge_entity::Status::Identified
    );

    let veto = store
        .veto(entity_id, "wikidata", "Q131472411", None)
        .await
        .unwrap();
    assert_eq!(veto.withdrawn, 1);

    let entity = store.get(entity_id).await.unwrap();
    assert_eq!(
        entity.status,
        crate::model::knowledge_entity::Status::Provisional,
        "no live claim, no identification"
    );
    assert_eq!(entity.confidence, None);
}

/// The projection recomputes from what is left: the withdrawn claim's score goes, the other stays.
#[tokio::test]
async fn a_veto_drops_the_score_that_claim_earned_and_no_more() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let entity_id = fixtures::knowledge_entity_with_member(&db, "Nintendo").await;
    fixtures::identity(
        &db,
        entity_id,
        "wikidata",
        "Q8093",
        "wikidata",
        Some("exact_label"),
        Some(0.9),
    )
    .await;
    fixtures::identity(
        &db,
        entity_id,
        "x_twitter",
        "nintendo",
        crate::knowledge::evidence::PLATFORM_LINK_PROVIDER_ID,
        None,
        Some(0.7),
    )
    .await;
    store.update_projection(entity_id).await.unwrap();
    // The best claim's score.
    assert!((store.get(entity_id).await.unwrap().confidence.unwrap() - 0.9).abs() < 1e-6);

    store
        .veto(entity_id, "wikidata", "Q8093", None)
        .await
        .unwrap();
    store.replay_verdicts().await.unwrap();

    let entity = store.get(entity_id).await.unwrap();
    assert_eq!(
        entity.status,
        crate::model::knowledge_entity::Status::Identified,
        "the handle still identifies it"
    );
    assert!(
        (entity.confidence.unwrap() - 0.7).abs() < 1e-6,
        "no stale bonus: {entity:?}"
    );
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
    assert_eq!(
        logged[0].cause,
        crate::model::knowledge_verdicts::Cause::ProximityAuto
    );

    let report = store.replay_verdicts().await.expect("replay");
    assert_eq!(report, ReplayReport::default(), "nothing to replay");
}

/// Replay folds what a previous build had merged, by origin.
#[tokio::test]
async fn replay_folds_the_pair_again() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    let winner_origin = fixtures::origin(&db, winner).await;
    let loser_origin = fixtures::origin(&db, loser).await;
    store.merge(loser, winner, Cause::Repair).await.unwrap();

    // A rebuild: the graph is emptied, the observations and the verdict survive it.
    crate::knowledge::snapshot::reset(&db).await.unwrap();
    let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    fixtures::set_origin(&db, reborn_winner, winner_origin).await;
    fixtures::set_origin(&db, reborn_loser, loser_origin).await;

    let report = store.replay_verdicts().await.expect("replay");

    assert_eq!(report.applied, 1, "{report:?}");
    let tombstone = model::knowledge_entity::Entity::find_by_id(reborn_loser)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tombstone.merged_into_id.is_some(),
        "the recorded side folded"
    );
}

/// The model may fold a person's winner into something else. The decision still means "these two
/// are one", so the replay follows the tombstone and folds into wherever the winner ended up.
#[tokio::test]
async fn replay_follows_a_winner_the_model_folded_away() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    let winner_origin = fixtures::origin(&db, winner).await;
    let loser_origin = fixtures::origin(&db, loser).await;
    store.merge(loser, winner, Cause::Repair).await.unwrap();

    // A rebuild gives the pair back, then the model folds the person's winner into a third
    // entity: the winner is now a tombstone, and the loser is not.
    crate::knowledge::snapshot::reset(&db).await.unwrap();
    let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
    fixtures::set_origin(&db, reborn_winner, winner_origin).await;
    fixtures::set_origin(&db, reborn_loser, loser_origin).await;
    let heir = fixtures::knowledge_entity_with_member(&db, "Train Dreams (film)").await;
    store
        .merge(reborn_winner, heir, Cause::ProximityAuto)
        .await
        .unwrap();

    let report = store.replay_verdicts().await.expect("replay");

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

    let report = store.replay_verdicts().await.expect("replay");

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
    let origin = fixtures::origin(&db, only).await;
    model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
        generation: Set(1),
        winner_origin: Set(Some(origin)),
        loser_origin: Set(Some(origin + 1000)),
        ..Default::default()
    })
    .exec(&db.conn)
    .await
    .unwrap();

    let report = store.replay_verdicts().await.expect("replay");

    assert_eq!(report.gone, 1, "{report:?}");
    assert_eq!(report.applied, 0, "{report:?}");
}

/// A chain the schema cannot refuse — only self-reference is a `CHECK` — is an error, not erosion.
/// Reporting `None` would drop the decision as if the entity had vanished.
#[tokio::test]
async fn a_merge_cycle_fails_loudly() {
    db_or_skip!(db);
    let first = fixtures::knowledge_entity_with_member(&db, "Cycle A").await;
    let second = fixtures::knowledge_entity_with_member(&db, "Cycle B").await;
    for (id, target) in [(first, second), (second, first)] {
        model::knowledge_entity::Entity::update(model::knowledge_entity::ActiveModel {
            id: Set(id),
            merged_into_id: Set(Some(target)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
    }

    let error = super::VerdictStore::from_handle(&db)
        .canonical(first)
        .await
        .expect_err("a cycle must not read as erosion");
    assert!(error.to_string().contains("cycle"), "{error}");
}
