//! DB-backed checks for the stats queries. Uses the schema-isolated harness in
//! `test_support::test_db`; skipped when no Postgres is reachable.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::{database, model, test_support::test_db::test_db};

use super::*;

fn statement(sql: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values)
}

async fn seed(db: &database::DbHandle) -> i32 {
    let user = model::user::ActiveModel::builder()
        .set_username("stats-user")
        .set_password_hash("unused")
        .set_storage_shard("testshard")
        .insert(&db.conn)
        .await
        .unwrap();

    db.conn
        .execute_unprepared(
            r#"
INSERT INTO task_runs
  (logical_id, run_id, run_number, task_type, entity_owner_id, entity_type, entity_id,
   status_code, attempts, created_at, processing_started_at, success_duration_ms,
   last_error_duration_ms, last_error_code, updated_at)
VALUES
  -- five successes: queued waits 100..500 ms, processing 1000..5000 ms
  ('l-s1','r-s1',1,'illuminate',1,'capture',1,4,1,
   now() - interval '1 hour', now() - interval '1 hour' + interval '100 ms', 1000, NULL, NULL, now()),
  ('l-s2','r-s2',1,'illuminate',1,'capture',2,4,1,
   now() - interval '1 hour', now() - interval '1 hour' + interval '200 ms', 2000, NULL, NULL, now()),
  ('l-s3','r-s3',1,'illuminate',1,'capture',3,4,1,
   now() - interval '1 hour', now() - interval '1 hour' + interval '300 ms', 3000, NULL, NULL, now()),
  ('l-s4','r-s4',1,'illuminate',1,'capture',4,4,1,
   now() - interval '1 hour', now() - interval '1 hour' + interval '400 ms', 4000, NULL, NULL, now()),
  ('l-s5','r-s5',1,'illuminate',1,'capture',5,4,1,
   now() - interval '1 hour', now() - interval '1 hour' + interval '500 ms', 5000, NULL, NULL, now()),
  -- two terminal failures: 2 s queue wait, 300 s processing, code 504
  ('l-f1','r-f1',1,'illuminate',1,'capture',6,5,3,
   now() - interval '1 hour', now() - interval '1 hour' + interval '2 s', NULL, 300000, 504, now()),
  ('l-f2','r-f2',1,'illuminate',1,'capture',7,5,3,
   now() - interval '1 hour', now() - interval '1 hour' + interval '2 s', NULL, 300000, 504, now()),
  -- current backlog: two queued (2 h and 4 h old), one in progress (30 s)
  ('l-q1','r-q1',1,'illuminate',1,'capture',8,1,0, now() - interval '2 hours', NULL, NULL, NULL, NULL, now()),
  ('l-q2','r-q2',1,'illuminate',1,'capture',9,1,0, now() - interval '4 hours', NULL, NULL, NULL, NULL, now()),
  ('l-p1','r-p1',1,'illuminate',1,'capture',10,2,1,
   now() - interval '1 hour', now() - interval '30 seconds', NULL, NULL, NULL, now()),
  -- one failed run awaiting its next attempt: age measured from the failure
  ('l-r1','r-r1',1,'illuminate',1,'capture',11,3,1,
   now() - interval '1 hour', now() - interval '50 seconds', NULL, 4200, 504,
   now() - interval '45 seconds')
"#,
        )
        .await
        .unwrap();

    db.conn
        .execute_raw(statement(
            r#"
INSERT INTO inference_metas
  (user_id, task_run_id, attempt, task_type, provider, backend, model,
   duration_ms, tokens_in, tokens_out)
VALUES
  ($1,'r-s1',1,'illuminate','gemini','vertex','test-model',1000,100,10),
  ($1,'r-s2',1,'illuminate','gemini','vertex','test-model',2000,200,20),
  ($1,'r-s3',1,'illuminate','gemini','vertex','test-model',3000,300,30)
"#,
            vec![user.id.into()],
        ))
        .await
        .unwrap();

    user.id
}

fn close(actual: Option<f64>, expected: f64) {
    close_within(actual, expected, 1e-6);
}

fn close_within(actual: Option<f64>, expected: f64, tolerance: f64) {
    let actual = actual.expect("expected a percentile value");
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected} ± {tolerance}, got {actual}"
    );
}

#[tokio::test]
async fn queue_stats_report_known_percentiles_and_backlog() {
    let Some(test_db) = test_db().await else {
        return;
    };
    let db = test_db.handle();
    seed(&db).await;

    let response = queue_stats(&db, &[24], crate::config::TaskQueueBackend::Local)
        .await
        .unwrap();
    assert!(response.snapshot_suppressed);

    let backlog = response
        .backlog
        .iter()
        .find(|row| row.task_type == "illuminate")
        .expect("illuminate backlog row");
    assert_eq!(backlog.queued.n, 2);
    close_within(backlog.queued.p50, 3.0 * 3_600_000.0, 60_000.0);
    assert_eq!(backlog.in_progress.n, 1);
    close_within(backlog.in_progress.p50, 30_000.0, 60_000.0);
    assert_eq!(backlog.error_will_retry.n, 1);
    close_within(backlog.error_will_retry.p50, 45_000.0, 60_000.0);

    let window = &response.windows[0];
    assert_eq!(window.hours, 24);
    let queue = window
        .queues
        .iter()
        .find(|row| row.task_type == "illuminate")
        .expect("illuminate stats row");
    assert_eq!(queue.queue, "illuminate");

    assert_eq!(queue.queued_success.n, 5);
    close(queue.queued_success.p50, 300.0);
    close(queue.queued_success.p90, 460.0);

    assert_eq!(queue.processing_success.n, 5);
    close(queue.processing_success.p50, 3000.0);
    close(queue.processing_success.p99, 4960.0);

    assert_eq!(queue.queued_failure.n, 2);
    close(queue.queued_failure.p50, 2000.0);
    assert_eq!(queue.processing_failure.n, 2);
    close(queue.processing_failure.p90, 300_000.0);

    assert_eq!(
        queue.failure_codes,
        vec![FailureCodeCount {
            code: Some(504),
            count: 2
        }]
    );
}

#[tokio::test]
async fn failures_list_terminal_and_optional_retrying() {
    let Some(test_db) = test_db().await else {
        return;
    };
    let db = test_db.handle();
    seed(&db).await;
    db.conn
        .execute_unprepared(
            "UPDATE task_runs SET last_error_message = 'request terminated at 300s' \
             WHERE run_id = 'r-f1'",
        )
        .await
        .unwrap();

    let terminal = queue_failures(&db, 24, 10, false).await.unwrap();
    assert!(!terminal.include_retrying);
    assert_eq!(terminal.failures.len(), 2);
    assert!(
        terminal
            .failures
            .iter()
            .all(|failure| failure.status == "complete_failure")
    );
    assert!(
        terminal
            .failures
            .iter()
            .all(|failure| failure.error_code == Some(504))
    );
    assert!(
        terminal
            .failures
            .iter()
            .all(|failure| failure.queue == "illuminate")
    );
    assert!(
        terminal.failures.iter().any(|failure| {
            failure.error_message.as_deref() == Some("request terminated at 300s")
        })
    );

    let with_retrying = queue_failures(&db, 24, 10, true).await.unwrap();
    assert_eq!(with_retrying.failures.len(), 3);
    assert!(
        with_retrying
            .failures
            .iter()
            .any(|failure| failure.status == "error_will_retry")
    );

    let limited = queue_failures(&db, 24, 1, true).await.unwrap();
    assert_eq!(limited.failures.len(), 1);
}

#[tokio::test]
async fn inference_stats_report_duration_and_tokens() {
    let Some(test_db) = test_db().await else {
        return;
    };
    let db = test_db.handle();
    seed(&db).await;

    let response = inference_stats(&db, &[24]).await.unwrap();

    let window = &response.windows[0];
    assert_eq!(window.hours, 24);
    let inference = window
        .inferences
        .iter()
        .find(|row| row.task_type == "illuminate")
        .expect("illuminate inference row");
    assert_eq!(inference.model, "test-model");
    assert_eq!(inference.backend, "vertex");

    assert_eq!(inference.duration.n, 3);
    close(inference.duration.p50, 2000.0);
    close(inference.tokens_in.p50, 200.0);
    close(inference.tokens_out.p50, 20.0);
    // Thought tokens were never recorded, so the metric has no samples.
    assert_eq!(inference.tokens_thought.n, 0);
}
