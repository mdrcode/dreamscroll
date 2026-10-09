use std::time::Duration;

use argh::FromArgs;

use crate::{api, illumination, rest, task};

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
    let (run_id, wait, wait_seconds) = match args.command {
        TaskCommand::Illuminate(args) => {
            let run_id = state
                .rest_client
                .enqueue_illuminate(args.capture_id, illumination::IlluminationVersion::V1)
                .await?;
            (run_id, args.wait, args.wait_seconds)
        }
        TaskCommand::SearchIndex(args) => {
            let run_id = state
                .rest_client
                .enqueue_search_index(args.capture_id)
                .await?;
            (run_id, args.wait, args.wait_seconds)
        }
    };

    report_submission(&run_id);
    let status = if wait {
        wait_for_task_run(
            &state.rest_client,
            &run_id,
            Duration::from_secs(wait_seconds),
            Duration::from_secs(2),
        )
        .await?
    } else {
        state.rest_client.get_task_run(&run_id).await?
    };
    report_status(&status);
    Ok(())
}

pub(super) async fn wait_for_task_run(
    client: &rest::client::Client,
    run_id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<api::TaskRunInfo> {
    wait_for_task_run_with(
        run_id.to_string(),
        timeout,
        poll_interval,
        |run_id| async move { client.get_task_run(&run_id).await },
    )
    .await
}

async fn wait_for_task_run_with<F, Fut>(
    run_id: String,
    timeout: Duration,
    poll_interval: Duration,
    mut fetch_status: F,
) -> anyhow::Result<api::TaskRunInfo>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<api::TaskRunInfo>>,
{
    let deadline = tokio::time::Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow::anyhow!("wait timeout is too large"))?;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let status = tokio::time::timeout(remaining, fetch_status(run_id.clone()))
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting to fetch task-run status"))??;

        if is_settled(status.status) || tokio::time::Instant::now() >= deadline {
            return Ok(status);
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(status);
        }
        tokio::time::sleep(poll_interval.min(remaining)).await;
    }
}

fn is_settled(status: task::TaskRunStatus) -> bool {
    matches!(
        status,
        task::TaskRunStatus::SubmissionFailed
            | task::TaskRunStatus::CompleteSuccess
            | task::TaskRunStatus::CompleteFailure
    )
}

fn report_submission(run_id: &str) {
    println!("Task submitted");
    println!("- Run ID: {run_id}");
}

fn report_status(status: &api::TaskRunInfo) {
    println!("- Logical ID: {}", status.logical_id);
    println!("- Run number: {}", status.run_number);
    println!("- Attempts: {}", status.attempts);
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use chrono::Utc;

    use super::*;

    fn run_id() -> String {
        "0542b870-bf59-4f71-8a47-9724945d5f1b".to_string()
    }

    fn status(status: crate::task::TaskRunStatus) -> api::TaskRunInfo {
        api::TaskRunInfo {
            logical_id: "u7-illuminate-capture42".to_string(),
            task_type: "illuminate".to_string(),
            run_id: "0542b870-bf59-4f71-8a47-9724945d5f1b".to_string(),
            run_number: 3,
            entity_type: "capture".to_string(),
            entity_id: 42,
            result_entity_type: None,
            result_entity_id: None,
            status,
            attempts: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn wait_returns_immediately_for_settled_run() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let requested_run_id = run_id();
        let expected_run_id = requested_run_id.clone();
        let result = wait_for_task_run_with(
            requested_run_id,
            Duration::from_secs(5),
            Duration::from_millis(1),
            move |received_run_id| {
                assert_eq!(received_run_id, expected_run_id);
                fetch_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(status(crate::task::TaskRunStatus::CompleteSuccess)) }
            },
        )
        .await
        .unwrap();

        assert_eq!(result.status, crate::task::TaskRunStatus::CompleteSuccess);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wait_polls_until_run_settles() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let result = wait_for_task_run_with(
            run_id(),
            Duration::from_secs(5),
            Duration::ZERO,
            move |_| {
                let call = fetch_calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    Ok(status(if call == 0 {
                        crate::task::TaskRunStatus::Queued
                    } else {
                        crate::task::TaskRunStatus::CompleteFailure
                    }))
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(result.status, crate::task::TaskRunStatus::CompleteFailure);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn wait_returns_latest_nonterminal_status_at_timeout() {
        let result = wait_for_task_run_with(
            run_id(),
            Duration::ZERO,
            Duration::from_secs(2),
            |_| async { Ok(status(crate::task::TaskRunStatus::InProgress)) },
        )
        .await
        .unwrap();

        assert_eq!(result.status, crate::task::TaskRunStatus::InProgress);
    }

    #[tokio::test]
    async fn wait_bounds_a_stalled_status_request() {
        let result = wait_for_task_run_with(
            run_id(),
            Duration::from_millis(10),
            Duration::from_secs(2),
            |_| async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(status(crate::task::TaskRunStatus::Queued))
            },
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn wait_rejects_unrepresentably_large_timeout() {
        let result = wait_for_task_run_with(
            run_id(),
            Duration::from_secs(u64::MAX),
            Duration::from_secs(1),
            |_| async { Ok(status(crate::task::TaskRunStatus::Queued)) },
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[tokio::test]
    async fn wait_propagates_status_fetch_errors() {
        let result = wait_for_task_run_with(
            run_id(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            |_| async { anyhow::bail!("status fetch failed") },
        )
        .await;

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("status fetch failed")
        );
    }
}
