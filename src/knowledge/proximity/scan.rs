//! Deriving the batch: which pairs the graph currently holds twice.

use std::collections::HashSet;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};

use crate::knowledge::evidence::Evidence;
use crate::knowledge::scoring::Scoring;

use super::Basis;

/// Derive the candidate set and replace the table with it. Returns how many rows the batch holds.
///
/// `conn` must be the store's own connection: the replacement is committed on it, and the holders are
/// read through it. `scoring` is the store's policy, passed in rather than read from a facade.
pub async fn scan(conn: &DatabaseConnection, scoring: &Scoring) -> anyhow::Result<usize> {
    let evidence = Evidence::new(conn.clone());
    let identities = identities_with_several_holders(conn).await?;

    // Derive off the pool: Postgres is READ COMMITTED, so a read inside the transaction would see the
    // same thing. One question per pair, even when two identities connect the same two entities.
    // Unordered, so a change in which side is the holder cannot reopen it.
    let mut rows = Vec::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    for identity_id in identities {
        // Most substantiated first: the survivor, and the reading order.
        let holders = evidence.held_by_identity(identity_id).await?;
        let Some(holder) = holders.first() else {
            continue;
        };
        for other in holders.iter().skip(1) {
            if !seen.insert((holder.id.min(other.id), holder.id.max(other.id))) {
                continue;
            }
            rows.push(Row {
                holder_id: holder.id,
                counterpart_id: other.id,
                knowledge_identity_id: Some(identity_id),
                score: Some(scoring.pair.score(other, holder)),
            });
        }
    }

    // One transaction: the table is one scan's answer, never a mixture of two.
    let raised = conn
        .transaction(move |txn| Box::pin(async move { replace(txn, rows).await }))
        .await
        .map_err(|error| anyhow::anyhow!("proximity scan failed: {error}"))?;
    Ok(raised)
}

/// One candidate, before it becomes a row.
struct Row {
    holder_id: i32,
    counterpart_id: i32,
    knowledge_identity_id: Option<i32>,
    score: Option<f32>,
}

/// Empty the table and write one scan's batch, atomically.
async fn replace<C: ConnectionTrait>(conn: &C, rows: Vec<Row>) -> anyhow::Result<usize> {
    conn.execute_unprepared("DELETE FROM knowledge_proximities")
        .await?;
    let mut raised = 0;
    // The batch was emptied first, so each write is a plain insert.
    for row in &rows {
        raised += insert(conn, row).await?;
    }
    Ok(raised)
}

/// Record one candidate. The batch was emptied first, so this is a plain insert.
async fn insert<C: ConnectionTrait>(conn: &C, row: &Row) -> anyhow::Result<usize> {
    let result = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO knowledge_proximities \
               (holder_id, counterpart_id, knowledge_identity_id, basis, score) \
             VALUES ($1, $2, $3, $4, $5)",
            [
                row.holder_id.into(),
                row.counterpart_id.into(),
                row.knowledge_identity_id.into(),
                Basis::SharedId.as_str().to_string().into(),
                row.score.into(),
            ],
        ))
        .await?;
    Ok(result.rows_affected() as usize)
}

/// Identities held by two or more live entities: the only source of candidates today.
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
