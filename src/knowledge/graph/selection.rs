//! The graph's read view: identity selection, origin resolution, and the sub-views beneath it.
//!
//! Selection is a read over annotations, not a write to them. The provider filter is per authority:
//! an entity Wikidata answered is still pending for another resolver.
//!
//! [`GraphSelector`] is the public entry point. `KnowledgeGraph` owns writes and projection; the
//! selector owns the selects, so the facade does not grow a query method per question.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use crate::knowledge::identity::IdentityKey;
use crate::knowledge::ids::{EntityOriginId, ObservationId};
use crate::model;

/// A borrowed read view over the graph, created by [`KnowledgeGraph::selector`].
///
/// Cheap to copy; it carries only the connection. Entity/identity reads are here; verdict reads are
/// a sub-view ([`GraphSelector::verdicts`]) so the aggregate's queries stay together.
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

    /// Illuminations with at least one observation not yet linked, oldest first.
    pub async fn awaiting_link(&self) -> anyhow::Result<Vec<model::illumination::ModelEx>> {
        let unlinked = Query::select()
            .column(model::user_observed_entity::Column::IlluminationId)
            .from(model::user_observed_entity::Entity)
            .and_where(model::user_observed_entity::Column::KnowledgeEntityId.is_null())
            .to_owned();
        Ok(model::illumination::Entity::load()
            .filter(Expr::col(model::illumination::Column::Id).in_subquery(unlinked))
            .with(model::user_observed_entity::Entity)
            .order_by_asc(model::illumination::Column::Id)
            .all(self.conn)
            .await?)
    }

    /// Which of `entity_ids` this provider already annotated.
    pub async fn identified_by(
        &self,
        provider: &str,
        entity_ids: &[i32],
    ) -> anyhow::Result<std::collections::HashSet<i32>> {
        identified_by(self.conn, provider, entity_ids).await
    }

    /// Follow an origin through tombstones to the live entity id.
    pub async fn live_entity(&self, origin: EntityOriginId) -> anyhow::Result<Option<i32>> {
        resolve_live_entity(self.conn, origin.0.0).await
    }

    /// The origin an entity id was created from.
    pub async fn origin_of(&self, entity: i32) -> anyhow::Result<Option<EntityOriginId>> {
        let row = model::knowledge_entity::Entity::find_by_id(entity)
            .one(self.conn)
            .await?;
        Ok(row.map(|row| EntityOriginId(ObservationId(row.origin_observation_id))))
    }

    /// How many provider rows currently assert this key for the entity.
    pub async fn identity_count(&self, entity: i32, key: &IdentityKey) -> anyhow::Result<usize> {
        count_identity_rows(self.conn, entity, key).await
    }

    /// The keys this provider currently asserts for the entity.
    pub async fn identity_keys(
        &self,
        entity: i32,
        provider: &str,
    ) -> anyhow::Result<Vec<IdentityKey>> {
        let rows = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity))
            .filter(model::knowledge_identity::Column::Provider.eq(provider))
            .all(self.conn)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| IdentityKey {
                scheme: row.scheme,
                external_id: row.external_id,
            })
            .collect())
    }

    /// The verdict sub-view: durable decisions and their replay reads.
    pub fn verdicts(&self) -> super::VerdictSelector<'a> {
        super::VerdictSelector::new(self.conn)
    }
}

/// Follow the tombstone chain from an origin to the live entity id.
pub(super) async fn resolve_live_entity(
    conn: &DatabaseConnection,
    origin: i32,
) -> anyhow::Result<Option<i32>> {
    let Some(row) = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::OriginObservationId.eq(origin))
        .one(conn)
        .await?
    else {
        return Ok(None);
    };

    let mut current = row.id;
    for _ in 0..8 {
        let Some(row) = model::knowledge_entity::Entity::find_by_id(current)
            .one(conn)
            .await?
        else {
            return Ok(None);
        };
        match row.merged_into_id {
            Some(next) => current = next,
            None => return Ok(Some(current)),
        }
    }
    anyhow::bail!(
        "merged_into_id chain from knowledge entity {} exceeds 8 hops",
        row.id
    )
}

async fn count_identity_rows(
    conn: &DatabaseConnection,
    entity: i32,
    key: &IdentityKey,
) -> anyhow::Result<usize> {
    Ok(model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(entity))
        .filter(model::knowledge_identity::Column::Scheme.eq(&key.scheme))
        .filter(model::knowledge_identity::Column::ExternalId.eq(&key.external_id))
        .count(conn)
        .await? as usize)
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
pub(super) async fn identified_by(
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
