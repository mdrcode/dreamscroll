use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use argh::FromArgs;

use crate::illumination::IlluminationVersion;
use crate::{
    api, illumination, rest,
    task::{self, TaskRunStatus},
};

use super::ApiCmdState;

#[derive(FromArgs)]
#[argh(
    subcommand,
    name = "illuminate",
    description = "Run illumination inference for a capture"
)]
pub struct IlluminateArgs {
    #[argh(positional, description = "capture ID")]
    capture_id: i32,

    #[argh(option, long = "prompt-version", default = "String::from(\"v2\")")]
    #[argh(description = "illumination prompt version: v1 or v2 (default: v2)")]
    prompt_version: String,

    #[argh(option, long = "thinking-level")]
    #[argh(
        description = "reasoning effort: minimal, low, medium, or high (default: provider default)"
    )]
    thinking_level: Option<crate::llms::gemini::ThinkingLevel>,

    #[argh(option, long = "persist-projection", default = "true")]
    #[argh(
        description = "persist relational projection and enqueue search indexing (default: true)"
    )]
    persist_projection: bool,

    #[argh(option, long = "wait-seconds", default = "60")]
    #[argh(description = "maximum seconds to wait for task completion (default: 60)")]
    wait_seconds: u64,
}

pub async fn run(state: ApiCmdState, args: IlluminateArgs) -> anyhow::Result<()> {
    let prompt_version = parse_prompt_version(&args.prompt_version)?;
    eprintln!(
        "Enqueueing illumination for capture {} ({})...",
        args.capture_id,
        prompt_version.as_str()
    );
    let run_id = state
        .rest_client
        .enqueue_illuminate(
            args.capture_id,
            prompt_version,
            args.thinking_level,
            args.persist_projection,
        )
        .await?;
    eprintln!("Enqueued TaskRun {run_id}.");

    eprintln!("Polling for task completion...");
    let task_run_info = wait_for_task_run(
        &state.rest_client,
        &run_id,
        Duration::from_secs(args.wait_seconds),
        Duration::from_secs(2),
    )
    .await?;
    ensure_successful_run(&task_run_info)?;

    eprintln!("Fetching raw illumination result by TaskRun ID...");
    let raw = state.rest_client.get_illumination_raw(&run_id).await?;
    let markdown = render_illumination(prompt_version, raw)?;
    println!("{markdown}");
    Ok(())
}

async fn wait_for_task_run(
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

fn parse_prompt_version(value: &str) -> anyhow::Result<IlluminationVersion> {
    match value {
        "v1" => Ok(IlluminationVersion::V1),
        "v2" => Ok(IlluminationVersion::V2),
        _ => bail!("unsupported prompt version {value:?}; expected v1 or v2"),
    }
}

fn ensure_successful_run(run_info: &api::TaskRunInfo) -> anyhow::Result<()> {
    match run_info.status {
        TaskRunStatus::CompleteSuccess => Ok(()),
        TaskRunStatus::SubmissionFailed | TaskRunStatus::CompleteFailure => Err(anyhow!(
            "illumination TaskRun {} (logical ID {}, run number {}) ended with status {} after {} attempt(s)",
            run_info.run_id,
            run_info.logical_id,
            run_info.run_number,
            run_info.status,
            run_info.attempts
        )),
        other_status => Err(anyhow!(
            "timed out waiting for illumination TaskRun {} (logical ID {}, run number {}); latest status {} after {} attempt(s)",
            run_info.run_id,
            run_info.logical_id,
            run_info.run_number,
            other_status,
            run_info.attempts
        )),
    }
}

fn render_illumination(
    prompt_version: IlluminationVersion,
    content: serde_json::Value,
) -> anyhow::Result<String> {
    match prompt_version {
        IlluminationVersion::V1 => illumination::v1::Illumination::from_raw_json(content)
            .context("failed to parse raw v1 illumination result")
            .map(|result| result.to_markdown()),
        IlluminationVersion::V2 => illumination::v2::Illumination::from_raw_json(content)
            .context("failed to parse raw v2 illumination result")
            .map(|result| result.to_markdown()),
    }
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

    #[test]
    fn persist_projection_defaults_true_and_requires_a_bool() {
        let parse = |args: &[&str]| IlluminateArgs::from_args(&["illuminate"], args);
        assert!(parse(&["42"]).unwrap().persist_projection);
        assert!(
            parse(&["42", "--persist-projection", "true"])
                .unwrap()
                .persist_projection
        );
        assert!(
            !parse(&["42", "--persist-projection", "false"])
                .unwrap()
                .persist_projection
        );
        assert!(parse(&["42", "--persist-projection", "yes"]).is_err());
        assert!(parse(&["42", "--persist-projection"]).is_err());
    }

    #[test]
    fn thinking_level_is_optional_and_rejects_unsupported_values() {
        let parse = |args: &[&str]| IlluminateArgs::from_args(&["illuminate"], args);
        assert_eq!(parse(&["42"]).unwrap().thinking_level, None);
        for (value, expected) in [
            ("minimal", crate::llms::gemini::ThinkingLevel::Minimal),
            ("low", crate::llms::gemini::ThinkingLevel::Low),
            ("medium", crate::llms::gemini::ThinkingLevel::Medium),
            ("high", crate::llms::gemini::ThinkingLevel::High),
        ] {
            assert_eq!(
                parse(&["42", "--thinking-level", value])
                    .unwrap()
                    .thinking_level,
                Some(expected)
            );
        }
        assert!(parse(&["42", "--thinking-level", "ultra"]).is_err());
    }

    #[test]
    fn parses_supported_prompt_versions() {
        assert_eq!(parse_prompt_version("v1").unwrap(), IlluminationVersion::V1);
        assert_eq!(parse_prompt_version("v2").unwrap(), IlluminationVersion::V2);
        assert!(parse_prompt_version("v3").is_err());
    }

    #[test]
    fn renders_the_selected_illumination_schema_as_markdown() {
        let markdown = render_illumination(
            IlluminationVersion::V2,
            serde_json::json!({
                "summary": "A Reddit community",
                "details": "Memes about the NFC West.",
                "suggested_searches": [],
                "entities": [{
                    "name": "NFCWestMemeWar",
                    "type": "online_community",
                    "platform_link": {
                        "platform": "reddit",
                        "handle": "r/NFCWestMemeWar"
                    }
                }]
            }),
        )
        .unwrap();

        assert!(markdown.contains("## Summary\n\nA Reddit community"));
        assert!(markdown.contains("Platform: reddit; handle: r/NFCWestMemeWar"));
    }

    #[test]
    fn only_successful_task_runs_allow_result_fetching() {
        let status = |status| api::TaskRunInfo {
            logical_id: "u7-illuminate-capture42".to_string(),
            run_id: "3e6d5dd1-4350-49d4-9933-e2de78aab82d".to_string(),
            run_number: 1,
            task_type: "illuminate".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 42,
            status,
            attempts: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert!(ensure_successful_run(&status(TaskRunStatus::CompleteSuccess)).is_ok());
        assert!(
            ensure_successful_run(&status(TaskRunStatus::CompleteFailure))
                .unwrap_err()
                .to_string()
                .contains("ended with status")
        );
        assert!(
            ensure_successful_run(&status(TaskRunStatus::Queued))
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }

    #[test]
    fn renders_v1_results_with_the_legacy_account_section() {
        let markdown = render_illumination(
            IlluminationVersion::V1,
            serde_json::json!({
                "summary": "Ada Lovelace",
                "details": "A mathematician and writer.",
                "suggested_searches": [],
                "entities": [],
                "social_media_accounts": [{
                    "display_name": "Ada",
                    "handle": "@ada",
                    "platform": "x_twitter"
                }]
            }),
        )
        .unwrap();

        assert!(markdown.contains("## Social media accounts\n- **Ada** (`@ada`; x_twitter)"));
    }
}
