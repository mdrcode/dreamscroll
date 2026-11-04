//! Reconciliation: identities held by more than one entity, in two phases.
//!
//! Sharing an identity is a candidate, not a verdict. An album's article is the identity of the album
//! *and* covers each of its songs, so two holders may be one thing or may be a work and the things
//! inside it. Claims are many-to-one precisely so both readings can be stored.
//!
//! **Scan** reads the graph and only writes rows: every pair that shares an identity becomes a
//! proximity, with the score the linker would give it. It mutates nothing else and is idempotent, so it
//! can run after a link pass as well as after a resolve.
//!
//! **Merge** consumes the open rows and is the only mutating phase: it re-checks that both sides still
//! exist, merges the pairs that score as one thing, and leaves the rest open for a person. A pair the
//! same-illumination veto refuses stays open — that is a question, not an error.

use std::cmp::Ordering;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::evidence::Evidence;
use super::merge::{Cause, Outcome};
use super::score::{Scoring, Subject};
use super::Knowledge;

/// What one reconcile did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Proximities the scan recorded that were not there before.
    pub raised: usize,
    /// Pairs the merge phase folded.
    pub merged: usize,
    /// Pairs that looked like one thing and the shared-illumination rule refused, left open.
    pub refused: usize,
    /// Pairs whose sides had already been resolved by some other path.
    pub moot: usize,
    /// Pairs that share an identity and do not look like one thing.
    pub undecided: usize,
}

/// Record every proximity the graph implies, then fold the ones that are convincing.
pub(super) async fn reconcile(
    knowledge: &Knowledge,
    conn: &DatabaseConnection,
) -> anyhow::Result<Report> {
    let raised = scan(&knowledge.scoring, conn).await?.raised;
    let mut report = merge_open(knowledge, conn).await?;
    report.raised = raised;
    Ok(report)
}

/// Find pairs that share an identity and record them. Read-only against the graph.
pub(super) async fn scan(scoring: &Scoring, conn: &DatabaseConnection) -> anyhow::Result<Report> {
    let evidence = Evidence::new(conn.clone());
    let mut report = Report::default();

    for identity_id in identities_with_several_holders(conn).await? {
        // Most substantiated first: the survivor, and the reading order.
        let holders = evidence.held_by_identity(identity_id, &*scoring.worth).await?;
        let Some(holder) = holders.first() else {
            continue;
        };
        for other in holders.iter().skip(1) {
            let features = scoring
                .similarity
                .pair(&Subject::of_entity(other), holder);
            report.raised += evidence
                .raise(
                    holder.id,
                    other.id,
                    Some(identity_id),
                    crate::knowledge::evidence::Basis::SharedId,
                    Some(features.total),
                )
                .await?;
        }
    }
    Ok(report)
}

/// Consume the open proximities: fold what is convincing, leave the rest as questions.
pub(super) async fn merge_open(
    knowledge: &Knowledge,
    conn: &DatabaseConnection,
) -> anyhow::Result<Report> {
    let mut report = Report::default();
    for (a, b) in open_proximities(conn).await? {
        let (Ok(left), Ok(right)) = (knowledge.get(a).await, knowledge.get(b).await) else {
            knowledge.settle(a, b, "moot").await?;
            report.moot += 1;
            continue;
        };
        // A tombstone on either side means some other path already answered this.
        if left.merged_into_id.is_some() || right.merged_into_id.is_some() {
            knowledge.settle(a, b, "moot").await?;
            report.moot += 1;
            continue;
        }

        // The more substantiated side survives; ties go to the lower id.
        let scoring = &knowledge.scoring;
        let (winner, loser) = if scoring.worth.order(right.confidence, left.confidence)
            == Ordering::Greater
        {
            (right, left)
        } else {
            (left, right)
        };
        let features = scoring
            .similarity
            .pair(&Subject::of_entity(&loser), &winner);
        if !scoring.judge.same(&features) {
            report.undecided += 1;
            continue;
        }
        match knowledge
            .merge(loser.id, winner.id, Cause::IdentityConflict)
            .await?
        {
            Outcome::Merged => report.merged += 1,
            // The merge settles the row naming both ends as `merged`; nothing more to do.
            Outcome::AlreadyMerged => report.moot += 1,
            // A question, not an error: leave it open for a person.
            Outcome::RefusedSharedIllumination => report.refused += 1,
        }
    }
    Ok(report)
}

/// The open proximities as pairs, most likely first.
pub(crate) async fn open_proximities(conn: &DatabaseConnection) -> anyhow::Result<Vec<(i32, i32)>> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT holder_id, counterpart_id \
               FROM knowledge_proximities \
              WHERE verdict IS NULL \
              ORDER BY score DESC NULLS LAST, id",
        ))
        .await?;
    rows.iter()
        .map(|row| Ok((row.try_get_by_index(0)?, row.try_get_by_index(1)?)))
        .collect()
}

/// One row per proximity, readable: what the pair is, how close, and what was decided.
///
/// The `note` says why a pair is worth a second look — `same entity_type`, or blank — and is also the
/// grouping. Ordered for triage, not by score: the highest scores are the pairs that must *never*
/// merge (same name, different type: Nintendo the organisation and the brand sit at 0.74), while the
/// actionable ones are same-type pairs whose names differ, where the shared identity is all the
/// evidence there is. Those come first, then score descending.
pub async fn list(
    conn: &DatabaseConnection,
    include_settled: bool,
    limit: usize,
) -> anyhow::Result<
    Vec<(
        i32,
        String,
        Option<f32>,
        String,
        String,
        String,
        String,
        String,
    )>,
> {
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT p.id, \
                    coalesce(p.verdict, 'open'), \
                    p.score, \
                    CASE WHEN h.entity_type = c.entity_type \
                         THEN 'same entity_type' ELSE '' END, \
                    p.basis, \
                    h.name || ' [' || h.entity_type || ']', \
                    c.name || ' [' || c.entity_type || ']', \
                    coalesce(i.scheme || ':' || i.external_id, '') \
               FROM knowledge_proximities p \
               JOIN knowledge_entities h ON h.id = p.holder_id \
               JOIN knowledge_entities c ON c.id = p.counterpart_id \
               LEFT JOIN knowledge_identities i ON i.id = p.knowledge_identity_id \
              WHERE $1 OR p.verdict IS NULL \
              ORDER BY p.verdict IS NOT NULL, \
                       (h.entity_type = c.entity_type) DESC, \
                       p.score DESC NULLS LAST, \
                       p.id \
              LIMIT $2",
            [include_settled.into(), (limit as i64).into()],
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get_by_index(0)?,
                row.try_get_by_index(1)?,
                row.try_get_by_index(2)?,
                row.try_get_by_index(3)?,
                row.try_get_by_index(4)?,
                row.try_get_by_index(5)?,
                row.try_get_by_index(6)?,
                row.try_get_by_index(7)?,
            ))
        })
        .collect()
}

async fn identities_with_several_holders(conn: &DatabaseConnection) -> anyhow::Result<Vec<i32>> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT c.knowledge_identity_id \
               FROM knowledge_claims c \
               JOIN knowledge_entities e ON e.id = c.knowledge_entity_id \
              WHERE c.retracted_at IS NULL AND e.merged_into_id IS NULL \
              GROUP BY 1 HAVING count(*) > 1 \
              ORDER BY 1",
        ))
        .await?;
    rows.iter()
        .map(|row| Ok(row.try_get_by_index(0)?))
        .collect()
}

#[cfg(test)]
mod tests {
    use sea_orm::EntityTrait;

    use crate::model;

    use super::*;
    use crate::test_support::{fixtures, test_db};

    async fn claim_pairs(db: &crate::database::DbHandle, names: [&str; 2]) -> (i32, i32) {
        let first = fixtures::knowledge_entity_with_member(db, names[0]).await;
        let second = fixtures::knowledge_entity_with_member(db, names[1]).await;
        fixtures::claim(db, first, "wikidata", "Q1", "wikidata", None, None, "text").await;
        fixtures::claim(db, second, "wikidata", "Q1", "wikidata", None, None, "text").await;
        (first, second)
    }

    /// The scan records the pair and mutates nothing else.
    #[tokio::test]
    async fn scanning_records_a_shared_identity() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;

        let report = scan(&crate::knowledge::Scoring::heuristic(), &db.conn).await.expect("scan");

        assert_eq!(report.raised, 1, "{report:?}");
        let row = model::knowledge_proximities::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("one question, one row");
        assert!(row.verdict.is_none(), "nothing decided yet");
        assert!(
            row.score.unwrap() >= crate::knowledge::heuristic_score::MERGE_BAR,
            "{:?}",
            row.score
        );

        // Idempotent: the same pair is not raised twice.
        assert_eq!(scan(&crate::knowledge::Scoring::heuristic(), &db.conn).await.unwrap().raised, 0);
    }

    /// The merge phase folds the pair the scan recorded, and says so on the row.
    #[tokio::test]
    async fn merging_settles_the_row_it_answers() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
        scan(&crate::knowledge::Scoring::heuristic(), &db.conn).await.unwrap();

        let report = merge_open(&store, &db.conn).await.expect("merge");

        assert_eq!(report.merged, 1, "{report:?}");
        let settled = model::knowledge_proximities::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the row survives the merge");
        assert_eq!(settled.verdict.as_deref(), Some("merged"));
        assert!(settled.settled_at.is_some());
    }

    /// Pairs that do not look like one thing stay open, so a person can answer.
    #[tokio::test]
    async fn scanning_leaves_distinct_pairs_open() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        let (album, song) = claim_pairs(&db, ["Belfast, Maine", "The Fishing Fleet"]).await;

        scan(&crate::knowledge::Scoring::heuristic(), &db.conn).await.unwrap();
        let report = merge_open(&store, &db.conn).await.expect("merge");

        assert_eq!(report.merged, 0, "{report:?}");
        assert_eq!(report.undecided, 1, "{report:?}");
        assert_eq!(open_proximities(&db.conn).await.unwrap().len(), 1);

        // A person says they are two things.
        assert_eq!(store.settle(album, song, "distinct").await.unwrap(), 1);
        assert!(open_proximities(&db.conn).await.unwrap().is_empty());
    }
}
