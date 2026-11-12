//! Which entities a provider should be asked about.
//!
//! Selection is a read over annotations, not a write to them. Keeping it beside the store that
//! writes evidence made `Evidence` look like it owned policy; this type owns only the questions.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::database::DbHandle;
use crate::model;

/// The candidate questions a provider still owes.
pub struct IdentitySelection<'a> {
    conn: &'a sea_orm::DatabaseConnection,
}

impl<'a> IdentitySelection<'a> {
    pub fn from_connection(conn: &'a sea_orm::DatabaseConnection) -> Self {
        Self { conn }
    }

    pub fn from_handle(db: &'a DbHandle) -> Self {
        Self::from_connection(&db.conn)
    }

    /// Entities no configured authority has annotated yet, in the order a run should ask them.
    ///
    /// The filter is *authority*, not *any evidence* — an entity identified only by a platform
    /// handle is still a candidate, because a QID is what corroboration needs.
    ///
    /// Ordering and limit are the caller's policy, passed in: ascending is a rebuild, descending is
    /// "what just arrived", and a finite budget has to reach one of them deterministically.
    pub async fn awaiting_authority(
        &self,
        sources: &[&str],
        limit: Option<usize>,
        newest_first: bool,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        use model::knowledge_entity as entity;

        // Asked already: holds an annotation a configured authority made, rather than only the
        // observation's handle.
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
        Ok(query.all(self.conn).await?)
    }

    /// The entities in `entity_ids` this provider already annotated.
    ///
    /// A resolver may still be asked about an entity another authority answered; only its own prior
    /// annotation removes it.
    pub async fn annotated_by(
        &self,
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
            .all(self.conn)
            .await?;
        Ok(rows.into_iter().collect())
    }
}
