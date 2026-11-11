//! Deriving the batch: which pairs the graph currently holds twice.

use std::collections::HashSet;

use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter,
    QueryOrder, QuerySelect, RelationTrait, Set, TransactionTrait,
};

use crate::knowledge::evidence::Evidence;
use crate::knowledge::scoring::Scoring;
use crate::model;

use super::Basis;

/// Derive the candidate set and replace the table with it. Returns how many rows the batch holds.
///
/// `conn` must be the store's own connection: the replacement is committed on it, and the holders are
/// read through it. `scoring` is the store's policy, passed in rather than read from a facade.
pub async fn scan(conn: &DatabaseConnection, scoring: &Scoring) -> anyhow::Result<usize> {
    let evidence = Evidence::from_connection(conn.clone());
    let identities = identities_with_several_holders(conn).await?;

    // Derive off the pool: Postgres is READ COMMITTED, so a read inside the transaction would see the
    // same thing. One question per pair, even when two identities connect the same two entities.
    // Unordered, so a change in which side is the holder cannot reopen it.
    let mut rows = Vec::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    for identity_id in identities {
        // Most substantiated first: the survivor, and the reading order.
        let holders = evidence.holders_of_identity(identity_id).await?;
        let Some(holder) = holders.first() else {
            continue;
        };
        for other in holders.iter().skip(1) {
            if !seen.insert((
                std::cmp::min(holder.id, other.id),
                std::cmp::max(holder.id, other.id),
            )) {
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
    model::knowledge_proximities::Entity::delete_many()
        .exec(conn)
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
    model::knowledge_proximities::Entity::insert(model::knowledge_proximities::ActiveModel {
        holder_id: Set(row.holder_id),
        counterpart_id: Set(row.counterpart_id),
        knowledge_identity_id: Set(row.knowledge_identity_id),
        basis: Set(Basis::SharedId.as_str().to_string()),
        score: Set(row.score),
        ..Default::default()
    })
    .exec_without_returning(conn)
    .await?;
    Ok(1)
}

/// Identities held by two or more live entities: the only source of candidates today.
async fn identities_with_several_holders(conn: &DatabaseConnection) -> anyhow::Result<Vec<i32>> {
    let rows = model::knowledge_claims::Entity::find()
        .select_only()
        .column(model::knowledge_claims::Column::KnowledgeIdentityId)
        .filter(model::knowledge_claims::Column::RetractedAt.is_null())
        .join(
            JoinType::InnerJoin,
            model::knowledge_claims::Relation::KnowledgeEntity.def(),
        )
        .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
        .group_by(model::knowledge_claims::Column::KnowledgeIdentityId)
        .having(
            Expr::col((
                model::knowledge_claims::Entity,
                model::knowledge_claims::Column::KnowledgeIdentityId,
            ))
            .count()
            .gt(1),
        )
        .order_by_asc(model::knowledge_claims::Column::KnowledgeIdentityId)
        .into_tuple::<i32>()
        .all(conn)
        .await?;
    Ok(rows)
}
