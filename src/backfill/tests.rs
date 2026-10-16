use std::sync::Arc;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder, Statement,
    sea_query::Expr,
};

use super::illuminate_v2_backfill::ILLUMINATE_V2_BACKFILL_ID;
use super::*;
use crate::{api, auth, database, illumination, logic, model, task};

fn admin_context() -> auth::Context {
    auth::Context::from(auth::DreamscrollAuthUser::new_test_jwt(
        999,
        auth::JwtUserClaims {
            sub: "999".into(),
            username: "admin".into(),
            is_admin: true,
            exp: 9999999999,
            iat: 1000000000,
            storage_shard: "adminshard".into(),
        },
    ))
}

async fn captures(db: &database::DbHandle, count: i32) {
    let owner = model::user::ActiveModel::builder()
        .set_username("capture-owner")
        .set_password_hash("unused")
        .set_storage_shard("testshard")
        .insert(&db.conn)
        .await
        .unwrap();
    db.conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO captures (id, user_id, created_at, archived_at)
         SELECT id, $1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP FROM generate_series(1, $2) AS id",
            [owner.id.into(), count.into()],
        ))
        .await
        .unwrap();
}

fn request() -> BackfillPumpRequest {
    BackfillPumpRequest {
        backfill_id: ILLUMINATE_V2_BACKFILL_ID.into(),
        limit: None,
    }
}

async fn settle(db: &database::DbHandle, run_id: &str, status: task::TaskRunStatus) {
    model::task_run::Entity::update_many()
        .col_expr(
            model::task_run::Column::StatusCode,
            Expr::value(status.as_i32()),
        )
        .filter(model::task_run::Column::RunId.eq(run_id))
        .exec(&db.conn)
        .await
        .unwrap();
}

#[tokio::test]
async fn bounded_pumps_cover_all_captures_and_do_not_resubmit_failed_targets() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    captures(&db, 101).await;
    let master = Arc::new(task::TaskMaster::builder().db(db.clone()).build().unwrap());
    let client = api::AdminApiClient::new(db.clone(), master);
    let first = client
        .backfill_pump(&admin_context(), request())
        .await
        .unwrap();
    assert_eq!((first.selected_count, first.enqueued_count), (100, 100));
    assert!(first.errors.is_empty());
    let runs = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(ILLUMINATE_V2_BACKFILL_ID))
        .order_by_desc(model::task_run::Column::EntityId)
        .all(&db.conn)
        .await
        .unwrap();
    assert_eq!(
        runs.iter().map(|run| run.entity_id).collect::<Vec<_>>(),
        (2..=101).rev().collect::<Vec<_>>()
    );
    assert!(runs.iter().all(|run| run.entity_owner_id == 1));
    settle(&db, &runs[0].run_id, task::TaskRunStatus::CompleteFailure).await;
    settle(&db, &runs[1].run_id, task::TaskRunStatus::SubmissionFailed).await;

    let second = client
        .backfill_pump(&admin_context(), request())
        .await
        .unwrap();
    assert_eq!((second.selected_count, second.enqueued_count), (1, 1));
    let last = model::task_run::Entity::find()
        .filter(model::task_run::Column::EntityId.eq(1))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last.backfill_id.as_deref(), Some(ILLUMINATE_V2_BACKFILL_ID));
    let exhausted = client
        .backfill_pump(&admin_context(), request())
        .await
        .unwrap();
    assert_eq!(exhausted.selected_count, 0);
}

#[tokio::test]
async fn live_refusal_stays_eligible_and_status_includes_only_tagged_task_types() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    captures(&db, 1).await;
    let master = Arc::new(task::TaskMaster::builder().db(db.clone()).build().unwrap());
    let client = api::AdminApiClient::new(db.clone(), master.clone());
    let context = admin_context();
    let task = logic::illuminate::IlluminationTask::new(
        1,
        1,
        "test-model",
        illumination::IlluminationVersion::V2,
    );
    let task::SubmitOutcome::Enqueued { run_id, .. } = master
        .submit_illuminate(&context, task.clone(), None)
        .await
        .unwrap()
    else {
        panic!("live run refused");
    };
    let blocked = client.backfill_pump(&context, request()).await.unwrap();
    assert_eq!(
        (
            blocked.selected_count,
            blocked.refused_in_flight_count,
            blocked.enqueued_count
        ),
        (1, 1, 0)
    );
    assert!(
        client
            .backfill_status(&context, ILLUMINATE_V2_BACKFILL_ID)
            .await
            .unwrap()
            .counts
            .is_empty()
    );
    settle(&db, &run_id, task::TaskRunStatus::CompleteSuccess).await;
    assert_eq!(
        client
            .backfill_pump(&context, request())
            .await
            .unwrap()
            .enqueued_count,
        1
    );
    let parent = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(ILLUMINATE_V2_BACKFILL_ID))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap();
    settle(&db, &parent.run_id, task::TaskRunStatus::CompleteSuccess).await;
    master
        .submit_search_index(
            &context,
            logic::search_index::SearchIndexTask {
                capture_id: 1,
                entity_owner_id: 1,
            },
            Some(ILLUMINATE_V2_BACKFILL_ID),
        )
        .await
        .unwrap();
    master
        .submit_illuminate(&context, task, None)
        .await
        .unwrap();

    let counts = client
        .backfill_status(&context, ILLUMINATE_V2_BACKFILL_ID)
        .await
        .unwrap()
        .counts;
    assert_eq!(
        counts
            .iter()
            .map(|row| (row.task_type.as_str(), row.status, row.count))
            .collect::<Vec<_>>(),
        vec![
            ("illuminate", task::TaskRunStatus::CompleteSuccess, 1),
            ("search_index", task::TaskRunStatus::Queued, 1),
        ]
    );
}

#[tokio::test]
async fn backfill_operations_require_admin_and_a_registered_definition() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    let master = Arc::new(task::TaskMaster::builder().db(db.clone()).build().unwrap());
    let client = api::AdminApiClient::new(db, master);
    let user = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(1));
    assert_eq!(
        client
            .backfill_pump(&user, request())
            .await
            .unwrap_err()
            .status_code,
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .backfill_status(&user, ILLUMINATE_V2_BACKFILL_ID)
            .await
            .unwrap_err()
            .status_code,
        axum::http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .backfill_pump(
                &admin_context(),
                BackfillPumpRequest {
                    backfill_id: "unknown".into(),
                    limit: None,
                }
            )
            .await
            .unwrap_err()
            .status_code,
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .backfill_status(&admin_context(), "unknown")
            .await
            .unwrap_err()
            .status_code,
        axum::http::StatusCode::NOT_FOUND
    );
}

#[derive(Debug)]
struct FailOneQueue;

#[async_trait::async_trait]
impl task::TaskQueue<logic::illuminate::IlluminationTask> for FailOneQueue {
    async fn enqueue(
        &self,
        run: task::TaskRun<logic::illuminate::IlluminationTask>,
    ) -> anyhow::Result<()> {
        if run.task.capture_id == 2 {
            anyhow::bail!("queue rejected capture 2");
        }
        Ok(())
    }
}

#[tokio::test]
async fn a_partial_pump_failure_is_reported_and_not_automatically_resubmitted() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    captures(&db, 3).await;
    let master = Arc::new(
        task::TaskMaster::builder()
            .db(db.clone())
            .illuminate_queue(FailOneQueue)
            .build()
            .unwrap(),
    );
    let client = api::AdminApiClient::new(db, master);
    let response = client
        .backfill_pump(&admin_context(), request())
        .await
        .unwrap();
    assert_eq!((response.selected_count, response.enqueued_count), (3, 2));
    assert_eq!(
        response
            .errors
            .iter()
            .map(|error| error.entity_id)
            .collect::<Vec<_>>(),
        vec![2]
    );
    let counts = client
        .backfill_status(&admin_context(), ILLUMINATE_V2_BACKFILL_ID)
        .await
        .unwrap()
        .counts;
    assert_eq!(
        counts
            .iter()
            .map(|row| (row.status, row.count))
            .collect::<Vec<_>>(),
        vec![
            (task::TaskRunStatus::SubmissionFailed, 1),
            (task::TaskRunStatus::Queued, 2),
        ]
    );
    assert_eq!(
        client
            .backfill_pump(&admin_context(), request())
            .await
            .unwrap()
            .selected_count,
        0
    );
}

#[tokio::test]
async fn candidate_limits_bound_attempts_and_can_exceed_the_default() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    captures(&db, 105).await;
    let master = Arc::new(task::TaskMaster::builder().db(db.clone()).build().unwrap());
    let client = api::AdminApiClient::new(db.clone(), master.clone());
    let context = admin_context();
    let task::SubmitOutcome::Enqueued { run_id, .. } = master
        .submit_illuminate(
            &context,
            logic::illuminate::IlluminationTask::new(
                105,
                1,
                "test-model",
                illumination::IlluminationVersion::V2,
            ),
            None,
        )
        .await
        .unwrap()
    else {
        panic!("live run refused");
    };
    let limited_request = |limit| BackfillPumpRequest {
        limit: Some(limit),
        ..request()
    };

    let first = client
        .backfill_pump(&context, limited_request(2))
        .await
        .unwrap();
    assert_eq!(
        (
            first.selected_count,
            first.enqueued_count,
            first.refused_in_flight_count
        ),
        (2, 1, 1)
    );
    let rows = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(ILLUMINATE_V2_BACKFILL_ID))
        .all(&db.conn)
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.entity_id).collect::<Vec<_>>(),
        vec![104]
    );

    settle(&db, &run_id, task::TaskRunStatus::CompleteSuccess).await;
    let second = client
        .backfill_pump(&context, limited_request(1))
        .await
        .unwrap();
    assert_eq!((second.selected_count, second.enqueued_count), (1, 1));
    let third = client
        .backfill_pump(&context, limited_request(200))
        .await
        .unwrap();
    assert_eq!((third.selected_count, third.enqueued_count), (103, 103));
    assert!(third.errors.is_empty());
    assert_eq!(
        client
            .backfill_pump(&context, request())
            .await
            .unwrap()
            .selected_count,
        0
    );
}

#[tokio::test]
async fn a_zero_candidate_limit_is_rejected_without_consuming_a_target() {
    let Some(test_db) = crate::test_support::test_db::test_db().await else {
        return;
    };
    let db = test_db.handle();
    captures(&db, 1).await;
    let master = Arc::new(task::TaskMaster::builder().db(db.clone()).build().unwrap());
    let client = api::AdminApiClient::new(db, master);
    let error = client
        .backfill_pump(
            &admin_context(),
            BackfillPumpRequest {
                limit: Some(0),
                ..request()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code, axum::http::StatusCode::BAD_REQUEST);
    let next = client
        .backfill_pump(
            &admin_context(),
            BackfillPumpRequest {
                limit: Some(1),
                ..request()
            },
        )
        .await
        .unwrap();
    assert_eq!((next.selected_count, next.enqueued_count), (1, 1));
}
