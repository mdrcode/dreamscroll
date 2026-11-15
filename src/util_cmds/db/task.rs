use argh::FromArgs;
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};

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
}

/// Close `InProgress` runs whose worker is gone.
///
/// `decide_next_run` refuses a resubmission while the latest run is in flight, so
/// a run whose process died — the local queue is in memory and a restart drops
/// its work — blocks that logical task forever. Cloud Tasks redelivers a failed
/// delivery, so this is a local-topology repair.
#[derive(FromArgs)]
#[argh(subcommand, name = "fail_stale")]
#[argh(description = "Mark InProgress runs untouched for N minutes as CompleteFailure")]
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

/// Flips stale `InProgress` rows to `CompleteFailure`, which is the terminal state
/// that lets the next submission claim a new run number.
///
/// Direct-database tool like `set_admin`: holding the DB credentials is the gate,
/// so it does not prompt for an admin login.
pub async fn run(db: &database::DbHandle, args: TaskArgs) -> anyhow::Result<()> {
    match args.command {
        TaskCommand::FailStale(args) => fail_stale(db, args).await,
    }
}

async fn fail_stale(db: &database::DbHandle, args: FailStaleArgs) -> anyhow::Result<()> {
    let cutoff = Utc::now() - chrono::Duration::minutes(i64::from(args.minutes));
    let stale = stale_in_progress(db, cutoff).await?;
    if stale.is_empty() {
        println!(
            "No InProgress runs untouched for {} minutes.",
            args.minutes
        );
        return Ok(());
    }
    print_runs(&stale);

    if args.dry_run {
        println!(
            "Would fail {} run(s); rerun without --dry-run to write.",
            stale.len()
        );
        return Ok(());
    }

    let note = args.note.unwrap_or_else(|| {
        format!(
            "abandoned: InProgress with no update for {} minutes (`dreamscroll_db task fail_stale`)",
            args.minutes
        )
    });
    let run_ids: Vec<String> = stale.iter().map(|row| row.run_id.clone()).collect();
    let closed = model::task_run::Entity::update_many()
        .col_expr(
            model::task_run::Column::StatusCode,
            Expr::value(TaskRunStatus::CompleteFailure.as_i32()),
        )
        .col_expr(
            model::task_run::Column::LastErrorMessage,
            Expr::value(note.as_str()),
        )
        .filter(model::task_run::Column::RunId.is_in(run_ids))
        .exec_with_returning(&db.conn)
        .await?;

    println!("Failed {} run(s).", closed.len());
    Ok(())
}

/// Runs in `InProgress` whose row has not been touched since `cutoff`.
///
/// The write closes exactly these rows (by `run_id`), so a dry run cannot drift
/// from what the repair would touch.
async fn stale_in_progress(
    db: &database::DbHandle,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<Vec<model::task_run::Model>> {
    Ok(model::task_run::Entity::find()
        .filter(model::task_run::Column::StatusCode.eq(TaskRunStatus::InProgress.as_i32()))
        .filter(model::task_run::Column::UpdatedAt.lt(cutoff))
        .order_by_asc(model::task_run::Column::UpdatedAt)
        .all(&db.conn)
        .await?)
}

fn print_runs(rows: &[model::task_run::Model]) {
    let now = Utc::now();
    for row in rows {
        println!(
            "{}  {}  {}/{}  untouched {} min",
            row.run_id,
            row.task_type,
            row.entity_type,
            row.entity_id,
            (now - row.updated_at).num_minutes()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::task_run::ActiveModel, test_support::db_or_skip};

    #[tokio::test]
    async fn fails_only_stale_in_progress_runs() {
        db_or_skip!(db);
        let now = Utc::now();
        let two_hours_ago = now - chrono::Duration::hours(2);
        let stale = insert_run(&db, "stale", TaskRunStatus::InProgress, two_hours_ago).await;
        let fresh = insert_run(&db, "fresh", TaskRunStatus::InProgress, now).await;
        let queued = insert_run(&db, "queued", TaskRunStatus::Queued, two_hours_ago).await;

        let args = || FailStaleArgs {
            minutes: 30,
            dry_run: true,
            note: None,
        };
        fail_stale(&db, args()).await.unwrap();
        assert_eq!(
            status_of(&db, &stale.run_id).await,
            TaskRunStatus::InProgress.as_i32(),
            "a dry run must not write"
        );

        fail_stale(
            &db,
            FailStaleArgs {
                dry_run: false,
                ..args()
            },
        )
        .await
        .unwrap();

        let closed = model::task_run::Entity::find_by_id(stale.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(closed.status_code, TaskRunStatus::CompleteFailure.as_i32());
        assert!(
            closed
                .last_error_message
                .as_deref()
                .unwrap_or_default()
                .contains("abandoned"),
            "the repair records why the run will never finish"
        );
        assert_eq!(
            status_of(&db, &fresh.run_id).await,
            TaskRunStatus::InProgress.as_i32(),
            "a run inside the threshold is untouched"
        );
        assert_eq!(
            status_of(&db, &queued.run_id).await,
            TaskRunStatus::Queued.as_i32(),
            "only InProgress is in scope"
        );
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
