//! Proximities: two entities the graph thinks might be one thing, and the passes that raise and
//! settle them.
//!
//! Sharing an identity is a candidate, not a verdict. An album's article is the identity of the album
//! *and* covers each of its songs, so two holders may be one thing or may be a work and the things
//! inside it. Claims are many-to-one precisely so both readings can be stored.
//!
//! [`scan`] reads the graph and only writes rows. [`merge`] consumes the open ones and is the only
//! mutating pass. Both are idempotent — a pair is recorded once, for ever, so a settled verdict
//! sticks — and a pair the shared-illumination rule refuses stays open: a question, not an error.

use std::cmp::Ordering;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::evidence::{Basis, Evidence};
use super::merge::{Cause, Outcome};
use super::scoring::{Scoring, TotalCmp};
use super::Knowledge;

/// What one pass did.
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

/// Find pairs that share an identity and record them. Read-only against the graph.
pub(super) async fn scan(scoring: &Scoring, conn: &DatabaseConnection) -> anyhow::Result<Report> {
    let evidence = Evidence::new(conn.clone());
    let mut report = Report::default();

    for identity_id in identities_with_several_holders(conn).await? {
        // Most substantiated first: the survivor, and the reading order.
        let holders = evidence.held_by_identity(identity_id).await?;
        let Some(holder) = holders.first() else {
            continue;
        };
        for other in holders.iter().skip(1) {
            let score = scoring.pair.score(other, holder);
            report.raised += raise(
                conn,
                holder.id,
                other.id,
                Some(identity_id),
                Basis::SharedId,
                Some(score),
            )
            .await?;
        }
    }
    Ok(report)
}

/// Fold the open proximities that look like one thing, and leave the rest as questions.
pub(super) async fn merge(
    knowledge: &Knowledge,
    conn: &DatabaseConnection,
) -> anyhow::Result<Report> {
    let mut report = Report::default();
    for (a, b) in open(conn).await? {
        let (Ok(left), Ok(right)) = (knowledge.get(a).await, knowledge.get(b).await) else {
            settle(conn, a, b, "moot").await?;
            report.moot += 1;
            continue;
        };
        // A tombstone on either side means some other path already answered this.
        if left.merged_into_id.is_some() || right.merged_into_id.is_some() {
            settle(conn, a, b, "moot").await?;
            report.moot += 1;
            continue;
        }

        // The more substantiated side survives; ties go to the lower id.
        let scoring = &knowledge.scoring;
        let (winner, loser) = if right.confidence.total_cmp(&left.confidence) == Ordering::Greater {
            (right, left)
        } else {
            (left, right)
        };
        // A set of one is the pair question: is there a candidate that is this thing?
        if scoring
            .merge
            .rank(&loser, std::slice::from_ref(&winner))
            .is_empty()
        {
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

/// Record a proximity: two entities close enough to ask about. A pair is recorded once, for ever —
/// `ON CONFLICT DO NOTHING` is what makes a settled verdict stick. Returns 1 when a row was added.
pub(super) async fn raise(
    conn: &DatabaseConnection,
    holder_id: i32,
    counterpart_id: i32,
    knowledge_identity_id: Option<i32>,
    basis: Basis,
    score: Option<f32>,
) -> anyhow::Result<usize> {
    let result = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO knowledge_proximities \
               (holder_id, counterpart_id, knowledge_identity_id, basis, score) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT DO NOTHING",
            [
                holder_id.into(),
                counterpart_id.into(),
                knowledge_identity_id.into(),
                basis.as_str().to_string().into(),
                score.into(),
            ],
        ))
        .await?;
    Ok(result.rows_affected() as usize)
}

/// Settle the open proximities between two entities with a verdict — `distinct` when a person says
/// they are two things, which no automatic pass can conclude. Returns how many rows closed.
pub(super) async fn settle(
    conn: &DatabaseConnection,
    a: i32,
    b: i32,
    verdict: &str,
) -> anyhow::Result<u64> {
    let result = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE knowledge_proximities SET verdict = $1, settled_at = now() \
              WHERE verdict IS NULL \
                AND ((holder_id = $2 AND counterpart_id = $3) \
                  OR (holder_id = $3 AND counterpart_id = $2))",
            [verdict.into(), a.into(), b.into()],
        ))
        .await?;
    Ok(result.rows_affected())
}

/// The open proximities as pairs, most likely first.
async fn open(conn: &DatabaseConnection) -> anyhow::Result<Vec<(i32, i32)>> {
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

    fn scoring() -> Scoring {
        crate::knowledge::heuristic::Heuristic::scoring()
    }

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

        let report = scan(&scoring(), &db.conn).await.expect("scan");

        assert_eq!(report.raised, 1, "{report:?}");
        let row = model::knowledge_proximities::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("one question, one row");
        assert!(row.verdict.is_none(), "nothing decided yet");
        assert!(
            row.score.unwrap() >= crate::knowledge::heuristic::scoring::MERGE_BAR,
            "{:?}",
            row.score
        );

        // Idempotent: the same pair is not raised twice.
        assert_eq!(scan(&scoring(), &db.conn).await.unwrap().raised, 0);
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
        scan(&scoring(), &db.conn).await.unwrap();

        let report = merge(&store, &db.conn).await.expect("merge");

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

        scan(&scoring(), &db.conn).await.unwrap();
        let report = merge(&store, &db.conn).await.expect("merge");

        assert_eq!(report.merged, 0, "{report:?}");
        assert_eq!(report.undecided, 1, "{report:?}");
        assert_eq!(open(&db.conn).await.unwrap().len(), 1);

        // A person says they are two things.
        assert_eq!(settle(&db.conn, album, song, "distinct").await.unwrap(), 1);
        assert!(open(&db.conn).await.unwrap().is_empty());
    }
}
