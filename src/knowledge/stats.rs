//! Counts that tell us whether the graph is working.

use std::collections::HashSet;

use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::{database, model};

#[derive(Debug, Default)]
pub struct Stats {
    pub observations: u64,
    pub linked: u64,
    pub entities: u64,
    pub provisional: u64,
    pub identified: u64,
    pub contested: u64,
    /// Entities with a live authority claim — the number `resolve` moves.
    pub with_claim: u64,
    /// Entities whose projected platform presence is set. A member had one and `project` kept it.
    pub with_platform: u64,
    /// Distinct entities that *should* have one, counted from the observations. Always equal to
    /// `with_platform`; when it is not, projection has lost something.
    pub platform_from_members: u64,
    pub cache_found: u64,
    pub cache_empty: u64,
    pub cache_failed: u64,
    /// Rows past their expiry: not answers any more, and refetchable.
    pub cache_expired: u64,
}

impl Stats {
    /// `1 - distinct_entities / observations`: the share of observations that share a referent.
    pub fn duplicate_rate(&self) -> f64 {
        if self.observations == 0 {
            return 0.0;
        }
        1.0 - self.entities as f64 / self.observations as f64
    }
}

pub async fn stats(db: &database::DbHandle) -> anyhow::Result<Stats> {
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

    let mut stats = Stats {
        observations,
        linked,
        platform_from_members: platform_from_members.len() as u64,
        ..Default::default()
    };

    for entity in model::knowledge_entity::Entity::find().all(&db.conn).await? {
        // A tombstone is history, not an entity: it keeps the columns it had and its members have
        // moved. Counting it inflated every number here by the number of merges.
        if entity.merged_into_id.is_some() {
            continue;
        }
        stats.entities += 1;
        match entity.status.as_str() {
            "identified" => stats.identified += 1,
            "contested" => stats.contested += 1,
            _ => stats.provisional += 1,
        }
        if entity.platform.is_some() {
            stats.with_platform += 1;
        }
    }

    let mut with_claim: HashSet<i32> = HashSet::new();
    for row in model::knowledge_evidence::Entity::find()
        .filter(model::knowledge_evidence::Column::Kind.eq("claim"))
        .filter(model::knowledge_evidence::Column::Source.ne("platform"))
        .filter(model::knowledge_evidence::Column::RetractedAt.is_null())
        .all(&db.conn)
        .await?
    {
        with_claim.insert(row.knowledge_entity_id);
    }
    stats.with_claim = with_claim.len() as u64;

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
            _ => {}
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use crate::knowledge::link;
    use crate::test_support::fixtures;

    #[tokio::test]
    async fn reports_linked_share_and_duplicate_rate() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let user_id = fixtures::user(&db, "stats").await;
        for (index, capture_id) in [(0, fixtures::capture(&db, user_id).await),
                                    (1, fixtures::capture(&db, user_id).await)] {
            let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
            fixtures::entity(&db, user_id, capture_id, illumination_id, index, "Train Dreams").await;
        }
        link::link(&db).await.expect("build should succeed");
        let stats = super::stats(&db).await.expect("stats should succeed");

        assert_eq!(stats.observations, 2);
        assert_eq!(stats.linked, 2);
        assert_eq!(stats.entities, 1);
        assert_eq!(stats.provisional, 1);
        assert!((stats.duplicate_rate() - 0.5).abs() < 1e-9, "{stats:?}");
    }
}
