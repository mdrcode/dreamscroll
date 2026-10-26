use std::sync::Arc;

use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    sea_query::SelectStatement,
};

use crate::{auth, logic, model, task};

use super::{
    definition::{Backfill, BackfillTarget, TargetColumn},
    engine::pump_definition,
};

struct SelectedSparks;

impl Backfill for SelectedSparks {
    type Task = logic::spark::SparkTask;

    fn id(&self) -> &'static str {
        "selected-sparks-test"
    }

    fn targets(&self) -> SelectStatement {
        model::spark::Entity::find()
            .inner_join(model::user::Entity)
            .filter(model::user::Column::Username.eq("eligible"))
            .select_only()
            .column_as(model::spark::Column::Id, TargetColumn::EntityId)
            .column_as(model::spark::Column::UserId, TargetColumn::OwnerId)
            .into_query()
    }

    fn make_task(&self, target: BackfillTarget) -> Self::Task {
        logic::spark::SparkTask {
            spark_id: target.entity_id,
            entity_owner_id: target.owner_id,
            capture_ids: vec![1],
        }
    }
}

#[tokio::test]
async fn backfill_supports_a_filtered_noncapture_relation_and_excludes_before_limiting() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    let owner = model::user::ActiveModel::builder()
        .set_username("eligible")
        .set_password_hash("unused")
        .set_storage_shard("eligible")
        .insert(&db.conn)
        .await
        .unwrap();
    let excluded_owner = model::user::ActiveModel::builder()
        .set_username("excluded")
        .set_password_hash("unused")
        .set_storage_shard("excluded")
        .insert(&db.conn)
        .await
        .unwrap();
    model::capture::ActiveModel::builder()
        .set_id(1)
        .set_user_id(owner.id)
        .set_created_at(chrono::Utc::now())
        .insert(&db.conn)
        .await
        .unwrap();
    model::spark::Entity::insert_many((1..=101).map(|id| model::spark::ActiveModel {
        id: Set(id),
        user_id: Set(owner.id),
        ..Default::default()
    }))
    .exec(&db.conn)
    .await
    .unwrap();
    model::spark::ActiveModel::builder()
        .set_id(1000)
        .set_user_id(excluded_owner.id)
        .insert(&db.conn)
        .await
        .unwrap();
    let master = Arc::new(task::TaskMaster::builder().db(db.clone()).build().unwrap());
    let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(owner.id));
    let definition = SelectedSparks;

    // Neither another task/entity type nor another backfill is a progress marker.
    master
        .submit_search_index(
            &context,
            logic::search_index::SearchIndexTask {
                capture_id: 1,
                entity_owner_id: owner.id,
            },
            Some(definition.id()),
        )
        .await
        .unwrap();
    let task::SubmitOutcome::Enqueued { run_id, .. } = master
        .submit_spark(
            &context,
            definition.make_task(BackfillTarget {
                entity_id: 101,
                owner_id: owner.id,
            }),
            Some("another-backfill"),
        )
        .await
        .unwrap()
    else {
        panic!("prior backfill refused");
    };
    model::task_run::Entity::update_many()
        .col_expr(
            model::task_run::Column::StatusCode,
            sea_orm::sea_query::Expr::value(task::TaskRunStatus::CompleteSuccess.as_i32()),
        )
        .filter(model::task_run::Column::RunId.eq(run_id))
        .exec(&db.conn)
        .await
        .unwrap();

    let first = pump_definition(&db, &definition, 100, false, |task, id| {
        master.submit_spark(&context, task, Some(id))
    })
    .await
    .unwrap();
    assert_eq!((first.selected_count, first.enqueued_count), (100, 100));
    assert!(first.errors.is_empty());
    let first_runs = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(definition.id()))
        .filter(model::task_run::Column::TaskType.eq("spark"))
        .order_by_desc(model::task_run::Column::EntityId)
        .all(&db.conn)
        .await
        .unwrap();
    assert_eq!(
        first_runs
            .iter()
            .map(|run| run.entity_id)
            .collect::<Vec<_>>(),
        (2..=101).rev().collect::<Vec<_>>()
    );
    assert!(
        first_runs
            .iter()
            .all(|run| run.entity_type == "spark" && run.entity_owner_id == owner.id)
    );
    assert_eq!(first_runs[0].run_number, 2);

    let second = pump_definition(&db, &definition, 100, false, |task, id| {
        master.submit_spark(&context, task, Some(id))
    })
    .await
    .unwrap();
    assert_eq!((second.selected_count, second.enqueued_count), (1, 1));
    let last = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(definition.id()))
        .filter(model::task_run::Column::TaskType.eq("spark"))
        .filter(model::task_run::Column::EntityId.eq(1))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last.entity_owner_id, owner.id);
    let exhausted = pump_definition(&db, &definition, 100, false, |task, id| {
        master.submit_spark(&context, task, Some(id))
    })
    .await
    .unwrap();
    assert_eq!(exhausted.selected_count, 0);
}
