use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use argh::FromArgs;

use crate::illumination::IlluminationVersion;
use crate::{api, illumination, task::TaskRunStatus};

use super::{ApiCmdState, task};

#[derive(FromArgs)]
#[argh(
    subcommand,
    name = "inference",
    description = "Enqueue inference tasks and display results"
)]
pub struct InferenceArgs {
    #[argh(subcommand)]
    command: InferenceCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum InferenceCommand {
    Illuminate(IlluminateArgs),
}

#[derive(FromArgs)]
#[argh(
    subcommand,
    name = "illuminate",
    description = "Run illumination inference for a capture"
)]
struct IlluminateArgs {
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

pub async fn run(state: ApiCmdState, args: InferenceArgs) -> anyhow::Result<()> {
    match args.command {
        InferenceCommand::Illuminate(args) => run_illumination(state, args).await,
    }
}

async fn run_illumination(state: ApiCmdState, args: IlluminateArgs) -> anyhow::Result<()> {
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
    let task_run_info = task::wait_for_task_run(
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
    use super::*;

    #[test]
    fn persist_projection_defaults_true_and_requires_a_bool() {
        let parse = |args: &[&str]| IlluminateArgs::from_args(&["inference", "illuminate"], args);
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
        let parse = |args: &[&str]| IlluminateArgs::from_args(&["inference", "illuminate"], args);
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
