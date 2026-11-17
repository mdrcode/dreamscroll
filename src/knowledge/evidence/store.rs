//! Where an identity annotation becomes a row, and every way it changes.
//!
//! A capture appends one annotation; the v2 resolve producer replaces a provider's set through graph
//! actions; a veto deletes one key across providers; a merge re-points them. Keeping the v1 writes
//! here means the veto rule, the provider scope and the uniqueness rule are one thing to read.

use std::cmp::Ordering;
use std::collections::HashSet;

use sea_orm::sea_query::{Expr, ExprTrait, OnConflict, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set, Statement,
};

use super::identity::{Identity, IdentityKey};
use crate::model;

/// What one provider replacement did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityWrite {
    /// Rows the replacement removed.
    pub removed: usize,
    /// Rows written. An idempotent append reports one even when the row already existed.
    pub written: usize,
    /// Keys a standing veto refused.
    pub vetoed: usize,
}

/// Identity annotations and the keys they cite.
///
/// Holds its own clone of the connection rather than borrowing one: `DatabaseConnection` is a pool
/// handle behind an `Arc`, so cloning is a pointer copy, and the type can be built once and kept in
/// a long-lived state.
#[derive(Clone)]
pub struct Evidence {
    conn: DatabaseConnection,
}

impl Evidence {
    pub fn from_connection(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    pub fn from_handle(db: &crate::database::DbHandle) -> Self {
        Self::from_connection(db.conn.clone())
    }

    /// Append one annotation, insert-if-absent, on the caller's connection.
    ///
    /// The capture path uses this, never a replacement: one illumination is not the complete history
    /// of an entity, so a later capture must not delete an earlier capture's handle. It writes where
    /// it is told rather than on its own pool handle, so the append commits with the attach that put
    /// the entity there — one illumination, one transaction.
    pub async fn add_identity<C: ConnectionTrait>(
        &self,
        conn: &C,
        entity_id: i32,
        provider: &str,
        identity: &Identity,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<IdentityWrite> {
        if vetoed_key(vetoes, entity_id, identity) {
            return Ok(IdentityWrite {
                removed: 0,
                written: 0,
                vetoed: 1,
            });
        }
        model::knowledge_identity::Entity::insert(active(entity_id, provider, identity))
            .on_conflict(
                OnConflict::columns([
                    model::knowledge_identity::Column::KnowledgeEntityId,
                    model::knowledge_identity::Column::Provider,
                    model::knowledge_identity::Column::Scheme,
                    model::knowledge_identity::Column::ExternalId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(conn)
            .await?;
        // A repeat is an idempotent no-op; the capture path only asks whether the write landed.
        Ok(IdentityWrite {
            removed: 0,
            written: 1,
            vetoed: 0,
        })
    }

    /// This entity's annotations, optionally for one provider, oldest first.
    pub async fn query_identities(
        &self,
        entity_id: i32,
        provider: Option<&str>,
    ) -> anyhow::Result<Vec<model::knowledge_identity::Model>> {
        let mut query = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity_id))
            .order_by_asc(model::knowledge_identity::Column::Id);
        if let Some(provider) = provider {
            query = query.filter(model::knowledge_identity::Column::Provider.eq(provider));
        }
        Ok(query.all(&self.conn).await?)
    }

    /// The live entities holding this key, most substantiated first.
    ///
    /// Asked here rather than written into the SQL: `merge` picks its survivor with the same
    /// comparison, and two spellings of "most substantiated" disagreed on `NULL` for as long as they
    /// existed.
    pub async fn query_entities_for_identity(
        &self,
        key: &IdentityKey,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        let holders = Query::select()
            .distinct()
            .column((
                model::knowledge_identity::Entity,
                model::knowledge_identity::Column::KnowledgeEntityId,
            ))
            .from(model::knowledge_identity::Entity)
            .and_where(
                Expr::col((
                    model::knowledge_identity::Entity,
                    model::knowledge_identity::Column::Scheme,
                ))
                .eq(&key.scheme),
            )
            .and_where(
                Expr::col((
                    model::knowledge_identity::Entity,
                    model::knowledge_identity::Column::ExternalId,
                ))
                .eq(&key.external_id),
            )
            .to_owned();

        let mut entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .filter(Expr::col(model::knowledge_entity::Column::Id).in_subquery(holders))
            .all(&self.conn)
            .await?;
        entities.sort_by(|a, b| confidence_cmp(&b.confidence, &a.confidence).then(a.id.cmp(&b.id)));
        Ok(entities)
    }

    /// Keys held by two or more live entities: the proximity scan's only source.
    pub(crate) async fn shared_identity_keys(&self) -> anyhow::Result<Vec<IdentityKey>> {
        let rows = self
            .conn
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT i.scheme, i.external_id \
                   FROM knowledge_identities i \
                   JOIN knowledge_entities e ON e.id = i.knowledge_entity_id \
                  WHERE e.merged_into_id IS NULL \
                  GROUP BY i.scheme, i.external_id \
                 HAVING count(DISTINCT i.knowledge_entity_id) > 1 \
                  ORDER BY i.scheme, i.external_id",
            ))
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(IdentityKey {
                    scheme: row.try_get_by_index(0)?,
                    external_id: row.try_get_by_index(1)?,
                })
            })
            .collect::<Result<Vec<_>, sea_orm::DbErr>>()
            .map_err(Into::into)
    }

    /// Delete every provider's annotation of `key` for this entity.
    pub(crate) async fn delete_identity(
        &self,
        entity_id: i32,
        key: &IdentityKey,
    ) -> anyhow::Result<usize> {
        Ok(model::knowledge_identity::Entity::delete_many()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity_id))
            .filter(model::knowledge_identity::Column::Scheme.eq(&key.scheme))
            .filter(model::knowledge_identity::Column::ExternalId.eq(&key.external_id))
            .exec(&self.conn)
            .await?
            .rows_affected as usize)
    }

    /// Re-point every annotation from `loser` to `winner`, dropping the duplicates first.
    ///
    /// A fold runs this on its transaction, so the annotations move with the observations and the
    /// tombstone. An annotation that duplicates the winner's would violate the uniqueness index on
    /// the way over, so it is deleted rather than moved: it is the same fact stated twice, and the
    /// winner's copy is the one kept.
    pub(crate) async fn move_identities<C: ConnectionTrait>(
        &self,
        conn: &C,
        loser: i32,
        winner: i32,
    ) -> anyhow::Result<()> {
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM knowledge_identities l USING knowledge_identities w \
             WHERE l.knowledge_entity_id = $1 AND w.knowledge_entity_id = $2 \
               AND l.provider = w.provider \
               AND l.scheme = w.scheme \
               AND l.external_id = w.external_id",
            [loser.into(), winner.into()],
        ))
        .await?;
        model::knowledge_identity::Entity::update_many()
            .col_expr(
                model::knowledge_identity::Column::KnowledgeEntityId,
                Expr::value(Some(winner)),
            )
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(loser))
            .exec(conn)
            .await?;
        Ok(())
    }
}

fn active(
    entity_id: i32,
    provider: &str,
    identity: &Identity,
) -> model::knowledge_identity::ActiveModel {
    model::knowledge_identity::ActiveModel {
        knowledge_entity_id: Set(entity_id),
        scheme: Set(identity.scheme.clone()),
        external_id: Set(identity.external_id.clone()),
        label: Set(identity.label.clone()),
        description: Set(identity.description.clone()),
        classes: Set(identity.classes.clone()),
        provider: Set(provider.to_string()),
        method: Set(identity.method.clone()),
        confidence: Set(identity.confidence),
        user_observed_entity_id: Set(identity.observation_id),
        ..Default::default()
    }
}

fn vetoed_key(
    vetoes: &HashSet<(i32, String, String)>,
    entity_id: i32,
    identity: &Identity,
) -> bool {
    vetoes.contains(&(
        entity_id,
        identity.scheme.clone(),
        identity.external_id.clone(),
    ))
}

/// `f32::total_cmp` over optional confidences, `None` least: an identity with no score
/// substantiates nothing, and a tie is not a win for either side.
fn confidence_cmp(a: &Option<f32>, b: &Option<f32>) -> Ordering {
    a.unwrap_or(f32::NEG_INFINITY)
        .total_cmp(&b.unwrap_or(f32::NEG_INFINITY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{db_or_skip, fixtures};

    fn identity(scheme: &str, external_id: &str) -> Identity {
        Identity {
            scheme: scheme.to_string(),
            external_id: external_id.to_string(),
            method: Some("test".to_string()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: None,
        }
    }

    fn vetoes() -> HashSet<(i32, String, String)> {
        HashSet::new()
    }

    /// The capture path appends: a second capture's handle must not delete the first's, and a
    /// repeat is one annotation.
    #[tokio::test]
    async fn appending_keeps_every_capture_handle_and_dedupes() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Tsoding").await;
        let evidence = Evidence::from_handle(&db);

        evidence
            .add_identity(
                &db.conn,
                entity,
                "illuminator",
                &identity("x_twitter", "tsoding"),
                &vetoes(),
            )
            .await
            .unwrap();
        evidence
            .add_identity(
                &db.conn,
                entity,
                "illuminator",
                &identity("youtube", "tsoding"),
                &vetoes(),
            )
            .await
            .unwrap();
        evidence
            .add_identity(
                &db.conn,
                entity,
                "illuminator",
                &identity("x_twitter", "tsoding"),
                &vetoes(),
            )
            .await
            .unwrap();

        let rows = evidence
            .query_identities(entity, Some("illuminator"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
    }

}
