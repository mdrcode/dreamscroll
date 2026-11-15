use argh::FromArgs;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr};

use crate::{database, model, task::TaskRunStatus};

#[derive(FromArgs)]
#[argh(subcommand, name = "task")]
#[argh(description = "Repair background task runs that no worker can finish")]
pub struct TaskArgs {
    #[argh(subcommand)]
    command: TaskCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum TaskCommand {
    FailStale(FailStaleArgs),
    DrainQueued(DrainQueuedArgs),
}

/// Close runs that a worker started and never finished.
///
/// `decide_next_run` refuses a resubmission while the latest run is in flight, so
/// a run whose process died — the local queue is in memory and a restart drops its
/// work — blocks that logical task forever. Cloud Tasks redelivers a failed
/// delivery, so both repairs below are local-topology tools.
#[derive(FromArgs)]
#[argh(subcommand, name = "fail_stale")]
#[argh(description = "Fail InProgress/ErrorWillRetry runs untouched for N minutes")]
struct FailStaleArgs {
    #[argh(option, default = "30")]
    #[argh(description = "staleness threshold in minutes (default: 30)")]
    minutes: u32,

    #[argh(switch)]
    #[argh(description = "print the runs that would change without writing")]
    dry_run: bool,

    #[argh(option)]
    #[argh(description = "message recorded on each run")]
    note: Option<String>,
}

/// Discard queue entries no worker ever received.
///
/// Distinct from `fail_stale`: nothing started, so there is no attempt to explain,
/// only a delivery that never happened.
#[derive(FromArgs)]
#[argh(subcommand, name = "drain_queued")]
#[argh(description = "Drop Queued runs no worker ever claimed")]
struct DrainQueuedArgs {
    #[argh(option, default = "30")]
    #[argh(description = "age threshold in minutes; 0 drains every queued run (default: 30)")]
    minutes: u32,

    #[argh(switch)]
    #[argh(description = "print the runs that would change without writing")]
    dry_run: bool,

    #[argh(option)]
    #[argh(description = "message recorded on each run")]
    note: Option<String>,
}

/// Both commands write the terminal `CompleteFailure`, the state that lets the
/// next submission claim a new run number. Direct-database tools like `set_admin`:
/// holding the DB credentials is the gate, so they do not prompt for a login.
pub async fn run(db: &database::DbHandle, args: TaskArgs) -> anyhow::Result<()> {
    match args.command {
        TaskCommand::FailStale(args) => {
            sweep(
                db,
                Sweep {
                    command: "fail_stale",
                    note_prefix: "abandoned",
                    statuses: &[
                        TaskRunStatus::InProgress,
                        TaskRunStatus::ErrorWillRetry,
                    ],
                    minutes: args.minutes,
                    dry_run: args.dry_run,
                    note: args.note.as_deref(),
                },
            )
            .await
        }
        TaskCommand::DrainQueued(args) => {
            sweep(
                db,
                Sweep {
                    command: "drain_queued",
                    note_prefix: "drained",
                    statuses: &[TaskRunStatus::Queued],
                    minutes: args.minutes,
                    dry_run: args.dry_run,
                    note: args.note.as_deref(),
                },
            )
            .await
        }
    }
}

struct Sweep<'a> {
    /// Subcommand name, recorded in the default note so the repair is auditable.
    command: &'a str,
    /// Verb the default note starts with: what the operator did to the run.
    note_prefix: &'a str,
    statuses: &'a [TaskRunStatus],
    minutes: u32,
    dry_run: bool,
    note: Option<&'a str>,
}

async fn sweep(db: &database::DbHandle, sweep: Sweep<'_>) -> anyhow::Result<()> {
    let cutoff = Utc::now() - chrono::Duration::minutes(i64::from(sweep.minutes));
    let stale = stale_runs(db, sweep.statuses, cutoff).await?;
    if stale.is_empty() {
        println!(
            "No {} runs untouched for {} minutes.",
            statuses_label(sweep.statuses),
            sweep.minutes
        );
        return Ok(());
    }
    print_runs(&stale);

    if sweep.dry_run {
        println!(
            "Would fail {} run(s); rerun without --dry-run to write.",
            stale.len()
        );
        return Ok(());
    }

    // One update per status so each note records what the run was; a single note
    // for the whole sweep could not name it.
    let mut closed = 0;
    for status in sweep.statuses {
        let run_ids: Vec<String> = stale
            .iter()
            .filter(|row| row.status_code == status.as_i32())
            .map(|row| row.run_id.clone())
            .collect();
        if run_ids.is_empty() {
            continue;
        }
        let note = sweep.note.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "{}: was {}, untouched for {} minutes (`dreamscroll_db task {}`)",
                sweep.note_prefix,
                status.as_str(),
                sweep.minutes,
                sweep.command
            )
        });
        let rows = model::task_run::Entity::update_many()
            .col_expr(
                model::task_run::Column::StatusCode,
                Expr::value(TaskRunStatus::CompleteFailure.as_i32()),
            )
            .col_expr(
                model::task_run::Column::LastErrorMessage,
                Expr::value(note),
            )
            .col_expr(
                model::task_run::Column::UpdatedAt,
                Expr::current_timestamp(),
            )
            .filter(model::task_run::Column::RunId.is_in(run_ids))
            .exec_with_returning(&db.conn)
            .await?;
        closed += rows.len();
    }

    println!("Failed {closed} run(s).");
    Ok(())
}

/// Runs in `statuses` whose row has not been touched since `cutoff`.
///
/// The write closes exactly these rows (by `run_id`), so a dry run cannot drift
/// from what the repair would touch.
async fn stale_runs(
    db: &database::DbHandle,
    statuses: &[TaskRunStatus],
    cutoff: DateTime<Utc>,
) -> anyhow::Result<Vec<model::task_run::Model>> {
    let status_codes: Vec<i32> = statuses.iter().map(TaskRunStatus::as_i32).collect();
    Ok(model::task_run::Entity::find()
        .filter(model::task_run::Column::StatusCode.is_in(status_codes))
        .filter(model::task_run::Column::UpdatedAt.lt(cutoff))
        .order_by_asc(model::task_run::Column::UpdatedAt)
        .all(&db.conn)
        .await?)
}

fn statuses_label(statuses: &[TaskRunStatus]) -> String {
    statuses
        .iter()
        .map(TaskRunStatus::as_str)
        .collect::<Vec<_>>()
        .join(" or ")
}

fn print_runs(rows: &[model::task_run::Model]) {
    let now = Utc::now();
    for row in rows {
        println!(
            "{}  {}  {}/{}  {}  untouched {} min",
            row.run_id,
            row.task_type,
            row.entity_type,
            row.entity_id,
            TaskRunStatus::from_i32(row.status_code)
                .map(|status| status.as_str().to_string())
                .unwrap_or_else(|_| row.status_code.to_string()),
            (now - row.updated_at).num_minutes()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::task_run::ActiveModel, test_support::db_or_skip};

    /// Both sweeps share one predicate, so one corpus covers the whole state
    /// space: which statuses each command claims, and the age boundary.
    #[tokio::test]
    async fn sweeps_close_only_their_own_statuses() {
        db_or_skip!(db);
        let now = Utc::now();
        let old = now - chrono::Duration::hours(2);
        let in_progress = insert_run(&db, "in_progress", TaskRunStatus::InProgress, old).await;
        let retrying = insert_run(&db, "retrying", TaskRunStatus::ErrorWillRetry, old).await;
        let queued = insert_run(&db, "queued", TaskRunStatus::Queued, old).await;
        let fresh_queued = insert_run(&db, "fresh_queued", TaskRunStatus::Queued, now).await;
        let fresh_in_progress =
            insert_run(&db, "fresh_in_progress", TaskRunStatus::InProgress, now).await;

        fail_stale(&db, true).await;
        assert_eq!(
            status_of(&db, &in_progress.run_id).await,
            TaskRunStatus::InProgress.as_i32(),
            "a dry run must not write"
        );

        fail_stale(&db, false).await;
        assert_eq!(
            status_of(&db, &in_progress.run_id).await,
            TaskRunStatus::CompleteFailure.as_i32()
        );
        assert_eq!(
            status_of(&db, &retrying.run_id).await,
            TaskRunStatus::CompleteFailure.as_i32(),
            "a run awaiting retry has no worker left to retry it"
        );
        assert_eq!(
            status_of(&db, &queued.run_id).await,
            TaskRunStatus::Queued.as_i32(),
            "fail_stale leaves queue entries to drain_queued"
        );
        assert_eq!(
            status_of(&db, &fresh_in_progress.run_id).await,
            TaskRunStatus::InProgress.as_i32(),
            "a run inside the threshold may still be alive"
        );

        drain_queued(&db, 30, false).await;
        assert_eq!(
            status_of(&db, &queued.run_id).await,
            TaskRunStatus::CompleteFailure.as_i32()
        );
        assert_eq!(
            status_of(&db, &fresh_queued.run_id).await,
            TaskRunStatus::Queued.as_i32(),
            "a just-submitted run is guarded by the age threshold"
        );

        let note = model::task_run::Entity::find_by_id(queued.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .last_error_message
            .unwrap_or_default();
        let drained = model::task_run::Entity::find_by_id(queued.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(
            drained.updated_at > old,
            "the repair stamps updated_at, like every other status transition"
        );
        assert!(
            note.starts_with("drained: was queued"),
            "the note names what the run was: {note}"
        );
    }

    /// Zero drains every queued row, whatever its age.
    #[tokio::test]
    async fn zero_minutes_drains_every_queued_run() {
        db_or_skip!(db);
        let queued = insert_run(
            &db,
            "just_queued",
            TaskRunStatus::Queued,
            Utc::now(),
        )
        .await;

        drain_queued(&db, 0, false).await;
        assert_eq!(
            status_of(&db, &queued.run_id).await,
            TaskRunStatus::CompleteFailure.as_i32()
        );
    }

    async fn fail_stale(db: &database::DbHandle, dry_run: bool) {
        sweep(
            db,
            Sweep {
                command: "fail_stale",
                note_prefix: "abandoned",
                statuses: &[TaskRunStatus::InProgress, TaskRunStatus::ErrorWillRetry],
                minutes: 30,
                dry_run,
                note: None,
            },
        )
        .await
        .unwrap();
    }

    async fn drain_queued(db: &database::DbHandle, minutes: u32, dry_run: bool) {
        sweep(
            db,
            Sweep {
                command: "drain_queued",
                note_prefix: "drained",
                statuses: &[TaskRunStatus::Queued],
                minutes,
                dry_run,
                note: None,
            },
        )
        .await
        .unwrap();
    }

    async fn insert_run(
        db: &database::DbHandle,
        name: &str,
        status: TaskRunStatus,
        updated_at: DateTime<Utc>,
    ) -> model::task_run::Model {
        ActiveModel::builder()
            .set_logical_id(format!("u1-illuminate-{name}"))
            .set_run_id(format!("{name}-run"))
            .set_run_number(1)
            .set_task_type("illuminate")
            .set_entity_owner_id(1)
            .set_entity_type("capture")
            .set_entity_id(700)
            .set_status_code(status.as_i32())
            .set_attempts(1)
            .set_updated_at(updated_at)
            .insert(&db.conn)
            .await
            .unwrap()
            .into()
    }

    async fn status_of(db: &database::DbHandle, run_id: &str) -> i32 {
        model::task_run::Entity::find()
            .filter(model::task_run::Column::RunId.eq(run_id))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .status_code
    }
}
