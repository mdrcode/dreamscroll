//! The knowledge entities one illumination references, in one join.
//!
//! An illumination's observations each name a thing; linking gives each one a `knowledge_entity_id`.
//! The read returns the distinct live entities *and* refuses when any observation is unlinked: an
//! observation with no entity means `link` has not run for this reading, or it failed on it, and a
//! partial batch would silently drop that observation from the resolve.

use std::collections::HashSet;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::api::ApiError;
use crate::database::DbHandle;
use crate::model;

/// The distinct live entities one illumination references, in observation order.
///
/// Errors if any of its observations has no entity yet — that is a link that has not finished, not
/// an empty batch.
pub async fn get_illumination_entities(
    db: &DbHandle,
    illumination_id: i32,
) -> Result<Vec<model::knowledge_entity::Model>, ApiError> {
    // `belongs_to` from the observation, so this is a LEFT JOIN: every observation comes back, with
    // its entity when it has one. `find_with_related` would need the reverse relation to exist.
    let rows = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::IlluminationId.eq(illumination_id))
        .find_also_related(model::knowledge_entity::Entity)
        .all(&db.conn)
        .await?;

    let mut entities = Vec::new();
    let mut seen = HashSet::new();
    let mut unlinked_observation_ids = Vec::new();
    for (observation, entity) in rows {
        match entity {
            Some(entity) if entity.merged_into_id.is_none() => {
                if seen.insert(entity.id) {
                    entities.push(entity);
                }
            }
            // An observation still pointing at a tombstone is a link that has not caught up with a
            // merge; resolving it would ask about a row nobody may annotate.
            Some(entity) => {
                return Err(ApiError::internal(anyhow::anyhow!(
                    "observation {} points at merged knowledge entity {}; re-link before resolving",
                    observation.id,
                    entity.id
                )));
            }
            None => unlinked_observation_ids.push(observation.id),
        }
    }

    if !unlinked_observation_ids.is_empty() {
        return Err(ApiError::internal(anyhow::anyhow!(
            "illumination {illumination_id} has unlinked observation(s) \
             {unlinked_observation_ids:?}; run link first"
        )));
    }

    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{fixtures, test_db};

    #[tokio::test]
    async fn one_join_reports_the_entities_and_whether_linking_finished() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "illumination_entities").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        // Before linking: the observation exists, the join finds no entity, and the read refuses.
        let before = get_illumination_entities(&db, illumination).await;
        assert!(before.is_err(), "an unlinked observation is an error");

        crate::test_support::link::link_all(&db).await.expect("link");

        let after = get_illumination_entities(&db, illumination).await.unwrap();
        assert_eq!(after.len(), 1);
    }
}
