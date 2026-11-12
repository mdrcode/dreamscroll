//! The two passes, end to end: derive a batch, then answer it.

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::knowledge::graph::KnowledgeGraph;
use crate::knowledge::linker::heuristic::Heuristic;
use crate::model;
use crate::test_support::{db_or_skip, fixtures};

use super::{auto_merge, scan};

async fn claim_pairs(db: &crate::database::DbHandle, names: [&str; 2]) -> (i32, i32) {
    let first = fixtures::knowledge_entity_with_member(db, names[0]).await;
    let second = fixtures::knowledge_entity_with_member(db, names[1]).await;
    fixtures::claim(db, first, "wikidata", "Q1", "wikidata", None, None).await;
    fixtures::claim(db, second, "wikidata", "Q1", "wikidata", None, None).await;
    (first, second)
}

async fn batch_size(db: &crate::database::DbHandle) -> u64 {
    model::knowledge_proximities::Entity::find()
        .count(&db.conn)
        .await
        .unwrap()
}

/// The scan records the pair and writes nothing else.
#[tokio::test]
async fn scanning_records_a_shared_identity() {
    db_or_skip!(db);
    claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;

    let raised = scan(&db.conn, &Heuristic::scoring()).await.expect("scan");

    assert_eq!(raised, 1);
    let row = model::knowledge_proximities::Entity::find()
        .one(&db.conn)
        .await
        .unwrap()
        .expect("one question, one row");
    // Two identical names and the same type: 0.6 * 1.0 + 0.1 * 1.0. Pinned, so a weight
    // change shows up here rather than as a silent shift in what folds.
    assert!(
        (row.score.unwrap() - 0.7).abs() < 1e-6,
        "{:?}",
        row.score
    );

    // The batch is replaced, not appended: a second scan leaves exactly one row.
    assert_eq!(scan(&db.conn, &Heuristic::scoring()).await.unwrap(), 1);
    assert_eq!(batch_size(&db).await, 1);
}

/// A pair can share two identities, and the scan reaches it once per identity. The batch is one
/// question per pair: `merge`'s `undecided` is read as questions owed, so a row per identity
/// would double it.
#[tokio::test]
async fn a_pair_sharing_two_identities_is_raised_once() {
    db_or_skip!(db);
    let (first, second) = claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
    for entity_id in [first, second] {
        fixtures::claim(
            &db,
            entity_id,
            "wikidata",
            "Q104919",
            "wikidata",
            None,
            None,
        )
        .await;
    }

    assert_eq!(scan(&db.conn, &Heuristic::scoring()).await.expect("scan"), 1, "one pair, one question");
    assert_eq!(batch_size(&db).await, 1);
}

/// The merge pass folds the pair and tombstones the loser; the row stays in the batch.
#[tokio::test]
async fn merging_folds_the_pair_it_answers() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
    scan(&db.conn, &Heuristic::scoring()).await.unwrap();

    let report = auto_merge(&db.conn, &store, &Heuristic::scoring(), true).await.expect("merge");

    assert_eq!(report.merged, 1, "{report:?}");
    assert_eq!(
        batch_size(&db).await,
        1,
        "the candidate is left in the batch for the next scan to replace"
    );
    let tombstones = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::MergedIntoId.is_not_null())
        .count(&db.conn)
        .await
        .unwrap();
    assert_eq!(tombstones, 1);
}

/// Pairs that do not look like one thing stay in the batch, so a person can answer.
#[tokio::test]
async fn merging_leaves_a_pair_it_cannot_claim() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    claim_pairs(&db, ["Belfast, Maine", "The Fishing Fleet"]).await;

    scan(&db.conn, &Heuristic::scoring()).await.unwrap();
    let report = auto_merge(&db.conn, &store, &Heuristic::scoring(), true).await.expect("merge");

    assert_eq!(report.merged, 0, "{report:?}");
    assert_eq!(report.undecided, 1, "{report:?}");
    assert_eq!(batch_size(&db).await, 1);
}

/// The preview reports the fold without writing it.
#[tokio::test]
async fn a_dry_run_reports_without_folding() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
    scan(&db.conn, &Heuristic::scoring()).await.unwrap();

    let report = auto_merge(&db.conn, &store, &Heuristic::scoring(), false)
        .await
        .expect("preview");

    assert_eq!(report.merged, 1, "{report:?}");
    let tombstones = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::MergedIntoId.is_not_null())
        .count(&db.conn)
        .await
        .unwrap();
    assert_eq!(tombstones, 0, "a dry run must not tombstone");
}

/// A distinct verdict keeps the pair out of the automatic pass.
///
/// The origins are pushed far from the entity ids on purpose: the fixture creates one user, one
/// capture, one illumination and one observation per entity, so a fresh schema makes
/// `entity.id == origin_observation_id`. That coincidence is what let the pass compare a
/// verdict's origins against entity ids and still look right.
#[tokio::test]
async fn a_distinct_pair_is_not_folded() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let (first, second) = claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
    fixtures::set_origin(&db, first, 100_001).await;
    fixtures::set_origin(&db, second, 100_002).await;
    scan(&db.conn, &Heuristic::scoring()).await.unwrap();
    assert!(store
        .distinct(first, second, Some("two different works"))
        .await
        .unwrap());

    let report = auto_merge(&db.conn, &store, &Heuristic::scoring(), true)
        .await
        .expect("merge");

    assert_eq!(report.merged, 0, "{report:?}");
    assert_eq!(report.kept_apart, 1, "{report:?}");
    let tombstones = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::MergedIntoId.is_not_null())
        .count(&db.conn)
        .await
        .unwrap();
    assert_eq!(tombstones, 0);
}

/// The decision is keyed by origin, so a rebuild — which renumbers every entity and leaves the
/// observations alone — cannot put the pair back in the automatic pass.
#[tokio::test]
async fn a_distinct_pair_survives_a_rebuild() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let (first, second) = claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
    fixtures::set_origin(&db, first, 100_001).await;
    fixtures::set_origin(&db, second, 100_002).await;
    assert!(store.distinct(first, second, None).await.unwrap());

    // A rebuild: the graph is emptied, the observations and the verdict survive it.
    crate::knowledge::snapshot::reset(&db).await.unwrap();
    let mut reborn = Vec::new();
    for origin in [100_001, 100_002] {
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::set_origin(&db, entity, origin).await;
        fixtures::claim(&db, entity, "wikidata", "Q1", "wikidata", None, None).await;
        reborn.push(entity);
    }
    assert!(
        reborn.iter().all(|id| *id != first && *id != second),
        "the rebuild renumbered the pair: {reborn:?} vs [{first}, {second}]"
    );

    scan(&db.conn, &Heuristic::scoring()).await.unwrap();
    let report = auto_merge(&db.conn, &store, &Heuristic::scoring(), true)
        .await
        .expect("merge");

    assert_eq!(report.merged, 0, "the decision still holds: {report:?}");
    assert_eq!(report.kept_apart, 1, "{report:?}");
}

/// The log of a distinct verdict is a pair, not a survivor.
#[tokio::test]
async fn a_distinct_verdict_is_idempotent() {
    db_or_skip!(db);
    let store = KnowledgeGraph::new(&db);
    let (first, second) = claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
    assert!(store.distinct(first, second, None).await.unwrap());
    assert!(!store.distinct(second, first, None).await.unwrap(), "direction-free");
}
