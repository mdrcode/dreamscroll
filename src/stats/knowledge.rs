//! Counts that tell us whether the knowledge graph is working.
//!
//! Read-only, over `model`: it touches no `knowledge` type, which is why it lives with the other
//! subsystem reports rather than inside the module it reports on.

use std::collections::HashSet;

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
    /// Candidates in the batch the last scan produced.
    pub proximities: u64,
    /// Entities appearing on either side of a candidate.
    pub entities_in_proximity: u64,
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
    /// Rows past their expiry: not answers any more, and refetchable. Kept for `PRUNE_GRACE`
    /// after expiry, then pruned by the next online resolve.
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
              WHERE c.retracted_at IS NULL AND i.asserted_by <> 'platform'",
        ),
    )
    .await?
    .into_iter()
    .map(|row| row.try_get_by_index::<i32>(0).map(|id| (id,)))
    .collect::<Result<_, _>>()?;
    stats.with_claim = with_claim.len() as u64;

    // One scan's candidates: the whole table.
    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM knowledge_proximities",
        ))
        .await?
        .expect("count always returns one row");
    stats.proximities = row.try_get_by_index::<i64>(0)? as u64;

    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM ( \
               SELECT holder_id AS entity FROM knowledge_proximities \
               UNION \
               SELECT counterpart_id FROM knowledge_proximities \
             ) sides",
        ))
        .await?
        .expect("count always returns one row");
    stats.entities_in_proximity = row.try_get_by_index::<i64>(0)? as u64;

    // The cache owns what an outcome means; this only reports what it counted.
    let cache = crate::fetch::cache::tally(&db.conn).await?;
    stats.cache_found = cache.found;
    stats.cache_empty = cache.empty;
    stats.cache_failed = cache.failed;
    stats.cache_degraded = cache.degraded;
    stats.cache_expired = cache.expired;

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn reports_linked_share_and_duplicate_rate() {
        db_or_skip!(db);

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

    #[test]
    fn duplicate_rate_of_an_empty_graph_is_zero() {
        assert_eq!(super::KnowledgeStats::default().duplicate_rate(), 0.0);
    }

    #[tokio::test]
    async fn projected_platform_presence_matches_the_members() {
        db_or_skip!(db);

        let user_id = fixtures::user(&db, "platform").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        let capture = fixtures::Capture {
            user_id,
            capture_id,
            illumination_id,
        };
        fixtures::observe(
            &db,
            &capture,
            0,
            "Some Person",
            None,
            "real_person",
            Some("instagram"),
            Some("@some_person"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let stats = super::knowledge_stats(&db)
            .await
            .expect("stats should succeed");
        assert_eq!(stats.with_platform, 1);
        assert_eq!(
            stats.with_platform, stats.platform_from_members,
            "projection lost presence a member had"
        );
    }
}
