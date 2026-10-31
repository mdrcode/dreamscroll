//! One-off copy of the live observation table into `user_knowledge_entities`.
//!
//! The application still writes `knowledge_user`; this table is the trial ground for the new
//! schema. Ids are preserved so a row can be compared across both, and the copy is idempotent, so
//! re-running picks up captures that have landed since.

use std::collections::HashSet;

use sea_orm::sea_query::OnConflict;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect, Set};

use crate::{database, model};

const BATCH: usize = 500;

#[derive(Debug, Default)]
pub struct Report {
    pub source: usize,
    pub already_present: usize,
    pub copied: usize,
}

pub async fn migrate(db: &database::DbHandle) -> anyhow::Result<Report> {
    let source = model::knowledge_entity::Entity::find()
        .order_by_asc(model::knowledge_entity::Column::Id)
        .all(&db.conn)
        .await?;

    let present: HashSet<i32> = model::user_knowledge_entity::Entity::find()
        .select_only()
        .column(model::user_knowledge_entity::Column::Id)
        .into_tuple::<i32>()
        .all(&db.conn)
        .await?
        .into_iter()
        .collect();

    let mut report = Report {
        source: source.len(),
        already_present: present.len(),
        copied: 0,
    };

    let missing: Vec<model::user_knowledge_entity::ActiveModel> = source
        .into_iter()
        .filter(|row| !present.contains(&row.id))
        .map(|row| model::user_knowledge_entity::ActiveModel {
            id: Set(row.id),
            user_id: Set(row.user_id),
            capture_id: Set(row.capture_id),
            illumination_id: Set(row.illumination_id),
            position: Set(row.position),
            created_at: Set(row.created_at),
            name: Set(row.name),
            entity_type: Set(row.entity_type),
            description: Set(row.description),
            search_query: Set(row.search_query),
            platform: Set(row.platform),
            platform_handle: Set(row.platform_handle),
            platform_display_name: Set(row.platform_display_name),
            core_id: Set(None),
        })
        .collect();

    let mut on_conflict = OnConflict::column(model::user_knowledge_entity::Column::Id);
    on_conflict.do_nothing();

    for batch in missing.chunks(BATCH) {
        model::user_knowledge_entity::Entity::insert_many(batch.to_vec())
            .on_conflict(on_conflict.clone())
            .exec_without_returning(&db.conn)
            .await?;
        report.copied += batch.len();
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;

    use super::*;
    use crate::model::user_knowledge_entity as observation;
    use crate::test_support::fixtures;

    #[tokio::test]
    async fn copies_every_observation_and_is_idempotent() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let user_id = fixtures::user(&db, "migrate").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        fixtures::entity(&db, user_id, capture_id, illumination_id, 0, "Train Dreams").await;

        let first = migrate(&db).await.expect("migrate should succeed");
        let second = migrate(&db).await.expect("migrate should be repeatable");

        assert_eq!(first.copied, 1, "{first:?}");
        assert_eq!(second.copied, 0, "{second:?}");
        assert_eq!(second.already_present, 1, "{second:?}");
        assert_eq!(
            observation::Entity::find().count(&db.conn).await.unwrap(),
            1
        );
    }
}
