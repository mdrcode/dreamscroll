//! Proximities: the candidates the last scan found, and the passes that produce and answer them.
//!
//! Sharing an identity is a candidate, not a verdict. An album's article is the identity of the album
//! *and* covers each of its songs, so two holders may be one thing, or may be a work and the things
//! inside it. Claims are many-to-one precisely so both readings can be stored.
//!
//! [`scan`] derives the table and replaces it wholesale, so what it holds is always "the candidates
//! as of the last scan" — no status, no history, no rows to settle. [`merge`] answers them: it folds
//! the pairs that score as one thing and leaves the rest, which are the questions still owed to a
//! person.

use std::cmp::Ordering;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};

use super::evidence::{Basis, Evidence};
use super::merge::{Cause, Outcome};
use super::scoring::TotalCmp;
use super::Knowledge;

/// What one merge did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Pairs the pass folded.
    pub merged: usize,
    /// Pairs that looked like one thing and the shared-illumination rule refused.
    pub refused: usize,
    /// Pairs whose sides had already been resolved by some other path.
    pub moot: usize,
    /// Pairs that share an identity and do not look like one thing.
    pub undecided: usize,
}

/// Derive the candidate set and replace the table with it. Returns how many rows the batch holds.
pub async fn scan(knowledge: &Knowledge) -> anyhow::Result<usize> {
    let conn = knowledge.conn();
    let evidence = Evidence::new(conn.clone());
    let identities = identities_with_several_holders(conn).await?;

    // One transaction: the table is one scan's answer, never a mixture of two.
    let txn = conn.begin().await?;
    txn.execute_unprepared("DELETE FROM knowledge_proximities")
        .await?;

    let mut raised = 0;
    for identity_id in identities {
        // Most substantiated first: the survivor, and the reading order.
        let holders = evidence.held_by_identity(identity_id).await?;
        let Some(holder) = holders.first() else {
            continue;
        };
        for other in holders.iter().skip(1) {
            let score = knowledge.scoring.pair.score(other, holder);
            raised += insert(
                &txn,
                holder.id,
                other.id,
                Some(identity_id),
                Basis::SharedId,
                Some(score),
            )
            .await?;
        }
    }
    txn.commit().await?;
    Ok(raised)
}

/// Fold the candidates that look like one thing, and leave the rest as questions.
pub async fn merge(knowledge: &Knowledge) -> anyhow::Result<MergeReport> {
    let mut report = MergeReport::default();
    for (a, b) in candidates(knowledge.conn()).await? {
        let (Ok(left), Ok(right)) = (knowledge.get(a).await, knowledge.get(b).await) else {
            // One side no longer names a row; the next scan will not raise the pair at all.
            report.moot += 1;
            continue;
        };
        // A tombstone on either side means some other path already answered this.
        if left.merged_into_id.is_some() || right.merged_into_id.is_some() {
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
            .merge(loser.id, winner.id, Cause::ProximityAuto)
            .await?
        {
            Outcome::Merged => report.merged += 1,
            Outcome::AlreadyMerged => report.moot += 1,
            // A question, not an error: the pair stays in the batch for a person to answer.
            Outcome::RefusedSharedIllumination => report.refused += 1,
        }
    }
    Ok(report)
}

/// Record one candidate. The batch was emptied first, so this is a plain insert.
async fn insert<C: ConnectionTrait>(
    conn: &C,
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
             VALUES ($1, $2, $3, $4, $5)",
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

/// The batch, most likely first.
async fn candidates(conn: &DatabaseConnection) -> anyhow::Result<Vec<(i32, i32)>> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT holder_id, counterpart_id \
               FROM knowledge_proximities \
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
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

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

    /// The scan records the pair and writes nothing else.
    #[tokio::test]
    async fn scanning_records_a_shared_identity() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;

        let raised = scan(&store).await.expect("scan");

        assert_eq!(raised, 1);
        let row = model::knowledge_proximities::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("one question, one row");
        assert!(
            row.score.unwrap() >= crate::knowledge::heuristic::scoring::MERGE_BAR,
            "{:?}",
            row.score
        );

        // The batch is replaced, not appended: a second scan leaves exactly one row.
        assert_eq!(scan(&store).await.unwrap(), 1);
        assert_eq!(candidates(&db.conn).await.unwrap().len(), 1);
    }

    /// The merge pass folds the pair and tombstones the loser; the row stays in the batch.
    #[tokio::test]
    async fn merging_folds_the_pair_it_answers() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        claim_pairs(&db, ["Train Dreams", "Train Dreams"]).await;
        scan(&store).await.unwrap();

        let report = merge(&store).await.expect("merge");

        assert_eq!(report.merged, 1, "{report:?}");
        assert_eq!(
            model::knowledge_proximities::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
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
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        claim_pairs(&db, ["Belfast, Maine", "The Fishing Fleet"]).await;

        scan(&store).await.unwrap();
        let report = merge(&store).await.expect("merge");

        assert_eq!(report.merged, 0, "{report:?}");
        assert_eq!(report.undecided, 1, "{report:?}");
        assert_eq!(candidates(&db.conn).await.unwrap().len(), 1);
    }
}
