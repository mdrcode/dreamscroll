//! The identity primitive every provider shares: which live entities still need work.
//!
//! Selection is a read over annotations, not a write to them. The provider filter is per authority:
//! an entity Wikidata answered is still pending for another resolver.
//!
//! [`GraphSelector`] is the public read view. `KnowledgeGraph2` owns writes and projection; the
//! selector owns the selects, so the facade does not grow a query method per question.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::model;

/// A borrowed read view over the graph, created by [`KnowledgeGraph2::selector`].
///
/// Cheap to copy; it carries only the connection. Queries remain methods until callers need to
/// compose filters, at which point a builder can grow here without changing the graph facade.
#[derive(Clone, Copy)]
pub struct GraphSelector<'a> {
    conn: &'a DatabaseConnection,
}

impl<'a> GraphSelector<'a> {
    pub(super) fn new(conn: &'a DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Live entities this provider has not answered, oldest or newest first.
    pub async fn awaiting_identity(
        &self,
        providers: &[&str],
        limit: Option<usize>,
        newest_first: bool,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        awaiting_identities(self.conn, providers, limit, newest_first).await
    }

    /// Which of `entity_ids` this provider already annotated.
    pub async fn annotated_by(
        &self,
        provider: &str,
        entity_ids: &[i32],
    ) -> anyhow::Result<std::collections::HashSet<i32>> {
        annotated_by(self.conn, provider, entity_ids).await
    }
}

/// Entities no configured provider has annotated yet, in the order a run should ask them.
pub(super) async fn awaiting_identities(
    conn: &DatabaseConnection,
    sources: &[&str],
    limit: Option<usize>,
    newest_first: bool,
) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
    use model::knowledge_entity as entity;

    let annotated = Query::select()
        .distinct()
        .column((
            model::knowledge_identity::Entity,
            model::knowledge_identity::Column::KnowledgeEntityId,
        ))
        .from(model::knowledge_identity::Entity)
        .and_where(
            Expr::col((
                model::knowledge_identity::Entity,
                model::knowledge_identity::Column::Provider,
            ))
            .is_in(
                sources
                    .iter()
                    .map(|source| source.to_string())
                    .collect::<Vec<_>>(),
            ),
        )
        .to_owned();

    let mut query = entity::Entity::find()
        .filter(entity::Column::MergedIntoId.is_null())
        .filter(Expr::col(entity::Column::Id).not_in_subquery(annotated))
        .order_by(
            entity::Column::Id,
            if newest_first {
                sea_orm::Order::Desc
            } else {
                sea_orm::Order::Asc
            },
        );
    if let Some(limit) = limit {
        query = query.limit(limit as u64);
    }
    Ok(query.all(conn).await?)
}

/// The entities in `entity_ids` this provider already annotated.
pub(super) async fn annotated_by(
    conn: &DatabaseConnection,
    provider: &str,
    entity_ids: &[i32],
) -> anyhow::Result<std::collections::HashSet<i32>> {
    if entity_ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let rows = model::knowledge_identity::Entity::find()
        .select_only()
        .column(model::knowledge_identity::Column::KnowledgeEntityId)
        .filter(model::knowledge_identity::Column::Provider.eq(provider))
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.is_in(entity_ids.to_vec()))
        .into_tuple::<i32>()
        .all(conn)
        .await?;
    Ok(rows.into_iter().collect())
}
