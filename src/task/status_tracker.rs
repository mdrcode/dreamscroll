use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{database, model};

use super::*;

/// True when a `DbErr` is a unique-constraint violation. The
/// `(envelope_id, run)` unique index is the real guard against a duplicate
/// submission, so this is expected, not exceptional.
fn is_unique_violation(err: &sea_orm::DbErr) -> bool {
    matches!(
        err.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    )
}

/// All direct reads/writes to `task_status`, shared by `TaskMaster` (writes) and
/// `StatusListener` (reads for SSE) so the SeaORM queries aren't duplicated.
#[derive(Clone)]
pub struct TaskStatusTracker {
    db: database::DbHandle,
}

impl TaskStatusTracker {
    pub fn new(db: database::DbHandle) -> Self {
        Self { db }
    }

    /// Insert the first row for a run. `Ok(false)` if `(envelope_id, run)`
    /// already exists — the *lost-a-race* case, where a concurrent submit
    /// claimed the run first. The unique index, not the read, is what makes
    /// correctness here.
    ///
    /// Requires the task payload (submitters and workers always hold it).
    pub async fn create_run<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<bool> {
        let db = &self.db;

        let Some(task) = envelope.task.as_ref() else {
            anyhow::bail!(
                "cannot record status for envelope without payload: {}",
                envelope.envelope_id
            );
        };

        let result = model::task_status::ActiveModel::builder()
            .set_task_type(T::task_type())
            .set_envelope_id(envelope.envelope_id.as_str())
            .set_run(envelope.run)
            .set_entity_type(T::entity_type())
            .set_entity_id(task.entity_id())
            .set_user_id(envelope.user_id)
            .set_status_code(status.as_i32())
            .set_attempts(attempts)
            .save(&db.conn)
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(err) if is_unique_violation(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Update the row for an existing `(envelope_id, run)`. No-op if missing.
    pub async fn update_run<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let db = &self.db;

        model::task_status::Entity::update_many()
            .col_expr(
                model::task_status::Column::StatusCode,
                sea_orm::sea_query::Expr::value(status.as_i32()),
            )
            .col_expr(
                model::task_status::Column::Attempts,
                sea_orm::sea_query::Expr::value(attempts),
            )
            .col_expr(
                model::task_status::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(chrono::Utc::now()),
            )
            .filter(model::task_status::Column::EnvelopeId.eq(envelope.envelope_id.as_str()))
            .filter(model::task_status::Column::Run.eq(envelope.run))
            .exec(&db.conn)
            .await?;

        Ok(())
    }

    /// The most recent run of a logical task, or `None` if it has never run.
    /// The decision input for submit: in flight → refuse, settled → rerun.
    pub async fn latest_run(
        &self,
        envelope_id: &str,
    ) -> anyhow::Result<Option<model::task_status::Model>> {
        let db = &self.db;

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
            .order_by_desc(model::task_status::Column::Run)
            .one(&db.conn)
            .await?;

        Ok(row)
    }

    /// The status row for one specific run, used by `begin_attempt`.
    pub async fn query_run_status(
        &self,
        envelope_id: &str,
        run: i32,
    ) -> anyhow::Result<Option<model::task_status::Model>> {
        let db = &self.db;

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
            .filter(model::task_status::Column::Run.eq(run))
            .one(&db.conn)
            .await?;

        Ok(row)
    }

    /// Incomplete task statuses for one entity, scoped by `user_id` (entity ids
    /// are not a security boundary). "Incomplete" = not `Completed`, which
    /// includes `ErrorExhausted`: failed work is still worth showing.
    ///
    /// Only the **latest run** per logical task is returned — a rerun supersedes
    /// the run before it.
    pub async fn query_incomplete_for_entity(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let db = &self.db;

        // TODO(REVISIT): wants a composite index on
        // (user_id, entity_type, entity_id, status_code).
        //
        // Deliberately no `status_code` filter: the latest run must be found
        // among *all* runs, else a stale incomplete run shadows a newer
        // completed one. See `incomplete_latest_runs`.
        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .filter(model::task_status::Column::EntityType.eq(entity_type))
            .filter(model::task_status::Column::EntityId.eq(entity_id))
            .all(&db.conn)
            .await?;

        Ok(incomplete_latest_runs(rows))
    }

    /// Every incomplete task status for a user. The user-level counterpart to
    /// `query_incomplete_for_entity`; also returns only the latest run each.
    pub async fn query_incomplete_for_user(
        &self,
        user_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let db = &self.db;

        // TODO(REVISIT): wants a composite index on (user_id, run).
        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .all(&db.conn)
            .await?;

        Ok(incomplete_latest_runs(rows))
    }
}

/// Reduce rows to the latest run per logical task, keeping those still
/// incomplete.
///
/// Collapse **before** filtering: filtering first lets an older incomplete run
/// shadow a newer completed one, reporting finished work as outstanding.
///
/// In Rust rather than SQL because the set is per-user (or per-entity) — small
/// enough that a correlated subquery or window function isn't worth it.
fn incomplete_latest_runs(rows: Vec<model::task_status::Model>) -> Vec<model::task_status::Model> {
    let mut latest: std::collections::HashMap<String, model::task_status::Model> =
        std::collections::HashMap::new();

    for row in rows {
        match latest.get(&row.envelope_id) {
            Some(existing) if existing.run >= row.run => {}
            _ => {
                latest.insert(row.envelope_id.clone(), row);
            }
        }
    }

    latest
        .into_values()
        .filter(|row| {
            StatusCode::from_i32(row.status_code)
                .map(|status| status.is_incomplete())
                .unwrap_or(true) // unknown status: surface it rather than hide it
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal task, so tracker tests don't depend on a real task type.
    #[derive(Debug, Clone, serde::Serialize)]
    struct TestTask {
        id: i32,
    }

    impl Task for TestTask {
        fn task_type() -> &'static str {
            "test"
        }
        fn entity_type() -> &'static str {
            "capture"
        }
        fn entity_id(&self) -> i32 {
            self.id
        }
    }

    fn envelope(user_id: i32, id: i32, run: i32) -> TaskEnvelope<TestTask> {
        TaskEnvelope::new(user_id, TestTask { id }, run)
    }

    fn row(envelope_id: &str, run: i32, status: StatusCode) -> model::task_status::Model {
        model::task_status::Model {
            id: 0,
            user_id: 1,
            envelope_id: envelope_id.to_string(),
            run,
            task_type: "test".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 1,
            status_code: status.as_i32(),
            attempts: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // --- `incomplete_latest_runs` (pure) ---

    #[test]
    fn collapse_keeps_the_latest_run() {
        let kept = incomplete_latest_runs(vec![
            row("a", 1, StatusCode::ErrorExhausted),
            row("a", 2, StatusCode::Queued),
        ]);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].run, 2, "the rerun supersedes the run before it");
    }

    #[test]
    fn collapse_drops_a_task_whose_latest_run_completed() {
        // An older failed run must not shadow a newer completed one, or finished
        // work would be reported as still outstanding.
        let kept = incomplete_latest_runs(vec![
            row("a", 1, StatusCode::ErrorExhausted),
            row("a", 2, StatusCode::Completed),
        ]);

        assert!(kept.is_empty());
    }

    #[test]
    fn collapse_returns_one_row_per_envelope() {
        let kept = incomplete_latest_runs(vec![
            row("a", 1, StatusCode::Queued),
            row("b", 1, StatusCode::InProgress),
            row("a", 2, StatusCode::Queued),
        ]);

        assert_eq!(kept.len(), 2, "one row per logical task");
    }

    #[test]
    fn collapse_keeps_failed_work_but_drops_completed() {
        let kept = incomplete_latest_runs(vec![
            row("a", 1, StatusCode::Completed),
            row("b", 1, StatusCode::ErrorExhausted),
        ]);

        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].envelope_id, "b",
            "the user still wants to see failures"
        );
    }

    #[test]
    fn collapse_surfaces_an_unreadable_status() {
        let mut unreadable = row("a", 1, StatusCode::Queued);
        unreadable.status_code = 99; // not a valid StatusCode discriminant

        let kept = incomplete_latest_runs(vec![unreadable]);

        assert_eq!(
            kept.len(),
            1,
            "an unknown status must not be silently hidden"
        );
    }

    // --- DB-backed ---

    #[tokio::test]
    async fn create_run_persists_the_full_identity() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskStatusTracker::new(db.handle());
        let env = envelope(1, 42, 1);

        assert!(
            tracker
                .create_run(&env, StatusCode::Queued, 0)
                .await
                .expect("create_run should succeed")
        );

        let stored = tracker
            .latest_run(&env.envelope_id)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(stored.run, 1);
        assert_eq!(stored.user_id, 1);
        assert_eq!(stored.entity_id, 42);
        assert_eq!(stored.entity_type, "capture");
        assert_eq!(stored.task_type, "test");
        assert_eq!(stored.status_code, StatusCode::Queued.as_i32());
        assert_eq!(stored.attempts, 0);
    }

    /// The unique index is what actually prevents a duplicate submission, so the
    /// tracker must report the conflict rather than propagate it as an error.
    #[tokio::test]
    async fn create_run_refuses_a_duplicate_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskStatusTracker::new(db.handle());
        let env = envelope(1, 42, 1);

        let first = tracker
            .create_run(&env, StatusCode::Queued, 0)
            .await
            .expect("first create_run should succeed");
        let second = tracker
            .create_run(&env, StatusCode::Queued, 0)
            .await
            .expect("a conflict must not be an error");

        assert!(first, "the first insert wins");
        assert!(!second, "the second loses the race");
    }

    #[tokio::test]
    async fn create_run_allows_a_new_run_of_the_same_task() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskStatusTracker::new(db.handle());

        assert!(
            tracker
                .create_run(&envelope(1, 42, 1), StatusCode::Completed, 1)
                .await
                .expect("run 1 should be created")
        );
        assert!(
            tracker
                .create_run(&envelope(1, 42, 2), StatusCode::Queued, 0)
                .await
                .expect("run 2 should be created")
        );

        let latest = tracker
            .latest_run(&envelope(1, 42, 1).envelope_id)
            .await
            .expect("query should succeed")
            .expect("a row should exist");

        assert_eq!(latest.run, 2, "latest_run picks the highest run");
    }

    #[tokio::test]
    async fn create_run_requires_a_payload() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskStatusTracker::new(db.handle());
        let mut env = envelope(1, 42, 1);
        env.task = None;

        let result = tracker.create_run(&env, StatusCode::Queued, 0).await;

        assert!(result.is_err(), "an envelope without a payload is a bug");
    }

    #[tokio::test]
    async fn update_run_touches_only_its_own_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskStatusTracker::new(db.handle());
        let run1 = envelope(1, 42, 1);
        let run2 = envelope(1, 42, 2);

        tracker
            .create_run(&run1, StatusCode::ErrorExhausted, 1)
            .await
            .expect("run 1 should be created");
        tracker
            .create_run(&run2, StatusCode::Queued, 0)
            .await
            .expect("run 2 should be created");

        tracker
            .update_run(&run1, StatusCode::InProgress, 7)
            .await
            .expect("update should succeed");

        let stored1 = tracker
            .query_run_status(&run1.envelope_id, 1)
            .await
            .expect("query should succeed")
            .expect("run 1 should exist");
        let stored2 = tracker
            .query_run_status(&run2.envelope_id, 2)
            .await
            .expect("query should succeed")
            .expect("run 2 should exist");

        assert_eq!(stored1.status_code, StatusCode::InProgress.as_i32());
        assert_eq!(stored1.attempts, 7);
        assert_eq!(
            stored2.status_code,
            StatusCode::Queued.as_i32(),
            "run 2 must be untouched"
        );

        // A run that was never written is simply absent, not an error.
        let missing = tracker
            .query_run_status(&run1.envelope_id, 99)
            .await
            .expect("query should succeed");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn query_incomplete_for_entity_is_user_scoped() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskStatusTracker::new(db.handle());

        tracker
            .create_run(&envelope(1, 42, 1), StatusCode::Queued, 0)
            .await
            .expect("create_run should succeed");

        let mine = tracker
            .query_incomplete_for_entity(1, "capture", 42)
            .await
            .expect("query should succeed");
        let theirs = tracker
            .query_incomplete_for_entity(2, "capture", 42)
            .await
            .expect("query should succeed");

        assert_eq!(mine.len(), 1);
        assert!(theirs.is_empty(), "entity ids are not a security boundary");
    }
}
