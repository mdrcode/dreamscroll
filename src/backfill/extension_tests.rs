use std::sync::Arc;

use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    sea_query::{Expr, SelectStatement},
};

use crate::{auth, database, logic, model, task, test_support::db_or_skip};

use super::{
    definition::{Backfill, BackfillTarget, TargetColumn},
    engine::pump_definition,
};

/// Records a completed spark run for `entity_id` tagged with `backfill_id`: a prior run the pump
/// treats as progress only when the tag and task type match its definition.
async fn spark_marker(
    db: &database::DbHandle,
    master: &task::TaskMaster,
    context: &auth::Context,
    owner_id: i32,
    entity_id: i32,
    backfill_id: &str,
) {
    let task::SubmitOutcome::Enqueued { run_id, .. } = master
        .submit_spark(
            context,
            logic::spark::SparkTask {
                spark_id: entity_id,
                entity_owner_id: owner_id,
                capture_ids: vec![1],
            },
            Some(backfill_id),
        )
        .await
        .unwrap()
    else {
        panic!("marker run for {entity_id} was refused");
    };
    model::task_run::Entity::update_many()
        .col_expr(
            model::task_run::Column::StatusCode,
            Expr::value(task::TaskRunStatus::CompleteSuccess.as_i32()),
        )
        .filter(model::task_run::Column::RunId.eq(run_id))
        .exec(&db.conn)
        .await
        .unwrap();
}

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
    db_or_skip!(db);
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
    model::spark::Entity::insert_many((1..=102).map(|id| model::spark::ActiveModel {
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
    spark_marker(&db, &master, &context, owner.id, 101, "another-backfill").await;

    // A run for this backfill and task type is a progress marker. Entity 100 sits inside the
    // descending batch, so if exclusion happened after the limit the batch would hold 99 rows.
    spark_marker(&db, &master, &context, owner.id, 100, definition.id()).await;

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
    // The 100 pumped runs plus the excluded entity's own marker. Excluding after the limit would
    // have left 99 pumped rows; the other owner's spark and the other task type are absent.
    assert_eq!(
        first_runs
            .iter()
            .map(|run| run.entity_id)
            .collect::<Vec<_>>(),
        (2..=102).rev().collect::<Vec<_>>()
    );
    assert!(
        first_runs
            .iter()
            .all(|run| run.entity_type == "spark" && run.entity_owner_id == owner.id)
    );
    // `logical_id` is owner+task, not backfill+task, so the pump's rerun of 101 is run 2.
    let resumed = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(definition.id()))
        .filter(model::task_run::Column::EntityId.eq(101))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.run_number, 2);

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
