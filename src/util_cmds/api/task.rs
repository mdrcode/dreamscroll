use std::time::Duration;

use argh::FromArgs;

use crate::{api, rest};

use super::ApiCmdState;

#[derive(FromArgs)]
#[argh(subcommand, name = "task")]
#[argh(description = "Submit a task through the REST API")]
pub struct TaskArgs {
    #[argh(subcommand)]
    command: TaskCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum TaskCommand {
    Illuminate(IlluminateArgs),
    SearchIndex(SearchIndexArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "illuminate")]
#[argh(description = "Submit a task for a capture")]
struct IlluminateArgs {
    #[argh(positional)]
    #[argh(description = "capture ID")]
    capture_id: i32,

    #[argh(switch)]
    #[argh(description = "wait for completion (up to 60 seconds by default)")]
    wait: bool,

    #[argh(option, long = "wait-seconds", default = "60")]
    #[argh(description = "maximum seconds to wait when --wait is set")]
    wait_seconds: u64,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "search_index")]
#[argh(description = "Submit a task for a capture")]
struct SearchIndexArgs {
    #[argh(positional)]
    #[argh(description = "capture ID")]
    capture_id: i32,

    #[argh(switch)]
    #[argh(description = "wait for completion (up to 60 seconds by default)")]
    wait: bool,

    #[argh(option, long = "wait-seconds", default = "60")]
    #[argh(description = "maximum seconds to wait when --wait is set")]
    wait_seconds: u64,
}

pub async fn run(state: ApiCmdState, args: TaskArgs) -> anyhow::Result<()> {
    let (identity, wait, wait_seconds) = match args.command {
        TaskCommand::Illuminate(args) => {
            let identity = state.client.enqueue_illuminate(args.capture_id).await?;
            (identity, args.wait, args.wait_seconds)
        }
        TaskCommand::SearchIndex(args) => {
            let identity = state.client.enqueue_search_index(args.capture_id).await?;
            (identity, args.wait, args.wait_seconds)
        }
    };

    report_submission(&identity.envelope_id, identity.run);
    let status = if wait {
        wait_for_task_run(
            &state.client,
            &identity,
            Duration::from_secs(wait_seconds),
            Duration::from_secs(2),
        )
        .await?
    } else {
        state
            .client
            .get_task_run(&identity.envelope_id, identity.run)
            .await?
    };
    report_status(&status);
    Ok(())
}

async fn wait_for_task_run(
    client: &rest::client::Client,
    identity: &api::TaskRunIdentity,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<api::TaskRunInfo> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let status = client
            .get_task_run(&identity.envelope_id, identity.run)
            .await?;
        if matches!(
            status.status,
            crate::task::TaskRunStatus::SubmissionFailed
                | crate::task::TaskRunStatus::CompleteSuccess
                | crate::task::TaskRunStatus::CompleteFailure
        ) || tokio::time::Instant::now() >= deadline
        {
            return Ok(status);
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(status);
        }
        tokio::time::sleep(poll_interval.min(remaining)).await;
    }
}

fn report_submission(envelope_id: &str, run: i32) {
    println!("Task submitted");
    println!("- Envelope ID: {envelope_id}");
    println!("- Run: {run}");
}

fn report_status(status: &api::TaskRunInfo) {
    println!("- Current status: {}", status.status);
    println!("- Attempts: {}", status.attempts);
}
