use sea_orm::{DeriveIden, FromQueryResult, IdenStatic, sea_query::SelectStatement};

use crate::task;

/// A code-defined batch workload: which entities to process and which Tasks
/// to create.
/// 
/// Backfills are defined in code, not the database. The adminstrator
/// advances progress of a backfill by calling the pump endpoint, which queries
/// the workload and enqueus a bounded batch of tasks
/// 
/// Conceptually, we think of a Backfill as a "left join" between the "work
/// to be done" (left side) and the "current status of the work" (right side).
/// The left side is defined by the Backfill definition (an implementation of
/// the `Backfill` trait). The right side is the TaskRun table, which records
/// which tasks have already been submitted and/or completed. The engine queries
/// the left side, excludes any targets that have a TaskRun for this backfill,
/// and submits a bounded batch of tasks to the TaskMaster. The engine does not
/// track the progress of the work; that is left to the TaskRun table and the
/// normal task execution machinery. The engine only tracks which targets have
/// been submitted, and prevents resubmission of those targets in future pumps.
///
/// TaskRuns track progress; even failed runs prevent resubmission by later pumps.
/// Execution and retries use the normal task machinery, and descendants inherit
/// the backfill ID. No remaining targets does not mean queued work has finished.
///
/// To add a workload, implement this trait and register its ID and typed
/// submission adapter in `engine.rs`.
pub(super) trait Backfill: Sync {
    /// The concrete task type; supplies the task and entity types for deduplication.
    type Task: task::Task;

    /// Stable execution ID used for progress, deduplication, and status reporting.
    /// Change it when changing the workload or intentionally rerunning it.
    fn id(&self) -> &'static str;

    /// Current candidates: an unpaginated query with one row per target and `i32`
    /// columns aliased as `entity_id` and `owner_id` (see `TargetColumn`). Joins and
    /// filters belong here; the engine excludes recorded work, orders by descending
    /// entity/owner ID, and limits the batch outside this query.
    fn targets(&self) -> SelectStatement;

    /// Build a complete Task, including execution parameters. Preserve the target's
    /// ID and owner; ownership comes from the query, not the requesting admin.
    fn make_task(&self, target: BackfillTarget) -> Self::Task;
}

/// Normalized target identity passed from the candidate query to the task factory.
#[derive(Clone, Copy, FromQueryResult)]
pub(super) struct BackfillTarget {
    pub entity_id: i32,
    pub owner_id: i32,
}

/// Required column aliases shared by candidate queries and the engine.
#[derive(Clone, Copy, Debug, DeriveIden)]
pub(super) enum TargetColumn {
    EntityId,
    OwnerId,
}

impl IdenStatic for TargetColumn {
    fn as_str(&self) -> &'static str {
        match self {
            Self::EntityId => "entity_id",
            Self::OwnerId => "owner_id",
        }
    }
}

/// Alias for the candidate subquery wrapped by the engine.
#[derive(DeriveIden)]
pub(super) struct BackfillTargets;
