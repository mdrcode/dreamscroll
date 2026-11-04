//! Counts that tell us whether the knowledge graph is working.
//!
//! Read-only, over `model`: it touches no `knowledge` type, which is why it lives with the other
//! subsystem reports rather than inside the module it reports on.

use std::collections::HashSet;

use chrono::Utc;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, PaginatorTrait, QueryFilter,
    Statement,
};

use crate::{database, model};

#[derive(Debug, Default)]
pub struct KnowledgeStats {
    pub observations: u64,
    pub linked: u64,
    pub entities: u64,
    pub provisional: u64,
    pub identified: u64,
    /// Proximities nobody has settled.
    pub proximities_open: u64,
    /// Proximities with a verdict.
    pub proximities_settled: u64,
    /// Entities on either side of an open proximity.
    pub entities_unsettled: u64,
    /// Entities with a live authority claim — the number `resolve` moves.
    pub with_claim: u64,
    /// Entities whose projected platform presence is set. A member had one and `project` kept it.
    pub with_platform: u64,
    /// Distinct entities that *should* have one, counted from the observations. Always equal to
    /// `with_platform`; when it is not, projection has lost something.
    pub platform_from_members: u64,
    pub cache_found: u64,
    /// Live answers that arrived, but not in a way worth trusting — the category most likely to hide a
    /// silent provider failure, so it is counted rather than folded into `found`.
    pub cache_degraded: u64,
    pub cache_empty: u64,
    pub cache_failed: u64,
    /// Rows past their expiry: not answers any more, and refetchable.
    pub cache_expired: u64,
}

impl KnowledgeStats {
    /// `1 - distinct_entities / observations`: the share of observations that share a referent.
    pub fn duplicate_rate(&self) -> f64 {
        if self.observations == 0 {
            return 0.0;
        }
        1.0 - self.entities as f64 / self.observations as f64
    }
}

pub async fn knowledge_stats(db: &database::DbHandle) -> anyhow::Result<KnowledgeStats> {
    let observations = model::user_observed_entity::Entity::find()
        .count(&db.conn)
        .await?;
    let linked = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
        .count(&db.conn)
        .await?;

    let platform_from_members: HashSet<i32> = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::Platform.is_not_null())
        .filter(model::user_observed_entity::Column::Platform.ne(""))
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
        .all(&db.conn)
        .await?
        .into_iter()
        .filter_map(|row| row.knowledge_entity_id)
        .collect();

    let mut stats = KnowledgeStats {
        observations,
        linked,
        platform_from_members: platform_from_members.len() as u64,
        ..Default::default()
    };

    for entity in model::knowledge_entity::Entity::find()
        .all(&db.conn)
        .await?
    {
        // A tombstone is history, not an entity: it keeps the columns it had and its members have
        // moved. Counting it inflated every number here by the number of merges.
        if entity.merged_into_id.is_some() {
            continue;
        }
        stats.entities += 1;
        match entity.status.as_str() {
            "identified" => stats.identified += 1,

            _ => stats.provisional += 1,
        }
        if entity.platform.is_some() {
            stats.with_platform += 1;
        }
    }

    // An authority claim is an edge to an identity someone other than the capture answered for.
    let with_claim: Vec<(i32,)> = sea_orm::ConnectionTrait::query_all_raw(
        &db.conn,
        sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT DISTINCT c.knowledge_entity_id \
               FROM knowledge_claims c \
               JOIN knowledge_identities i ON i.id = c.knowledge_identity_id \
              WHERE c.retracted_at IS NULL AND i.source <> 'platform'",
        ),
    )
    .await?
    .into_iter()
    .map(|row| row.try_get_by_index::<i32>(0).map(|id| (id,)))
    .collect::<Result<_, _>>()?;
    stats.with_claim = with_claim.len() as u64;

    // Proximities are a record with a verdict, not a status: count them where they live.
    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FILTER (WHERE verdict IS NULL), \
                    count(*) FILTER (WHERE verdict IS NOT NULL) \
               FROM knowledge_proximities",
        ))
        .await?
        .expect("count always returns one row");
    stats.proximities_open = row.try_get_by_index::<i64>(0)? as u64;
    stats.proximities_settled = row.try_get_by_index::<i64>(1)? as u64;

    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM ( \
               SELECT holder_id AS entity FROM knowledge_proximities WHERE verdict IS NULL \
               UNION \
               SELECT counterpart_id FROM knowledge_proximities WHERE verdict IS NULL \
             ) sides",
        ))
        .await?
        .expect("count always returns one row");
    stats.entities_unsettled = row.try_get_by_index::<i64>(0)? as u64;

    let now = Utc::now();
    for row in model::resolution_cache::Entity::find()
        .all(&db.conn)
        .await?
    {
        if row.expires_at <= now {
            stats.cache_expired += 1;
            continue;
        }
        match row.outcome.as_str() {
            "found" => stats.cache_found += 1,
            "empty" => stats.cache_empty += 1,
            "failed" => stats.cache_failed += 1,
            "degraded" => stats.cache_degraded += 1,
            _ => {}
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use crate::test_support::fixtures;

    #[tokio::test]
    async fn reports_linked_share_and_duplicate_rate() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let user_id = fixtures::user(&db, "stats").await;
        for (index, capture_id) in [
            (0, fixtures::capture(&db, user_id).await),
            (1, fixtures::capture(&db, user_id).await),
        ] {
            let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
            fixtures::entity(
                &db,
                user_id,
                capture_id,
                illumination_id,
                index,
                "Train Dreams",
            )
            .await;
        }
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        let stats = super::knowledge_stats(&db)
            .await
            .expect("stats should succeed");

        assert_eq!(stats.observations, 2);
        assert_eq!(stats.linked, 2);
        assert_eq!(stats.entities, 1);
        assert_eq!(stats.provisional, 1);
        assert!((stats.duplicate_rate() - 0.5).abs() < 1e-9, "{stats:?}");
    }
}
