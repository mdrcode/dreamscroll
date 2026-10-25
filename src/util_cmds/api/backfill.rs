use argh::FromArgs;

use crate::backfill;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "backfill")]
#[argh(description = "Pump and inspect code-defined backfills via admin API")]
pub struct BackfillArgs {
    #[argh(subcommand)]
    command: BackfillCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum BackfillCommand {
    Pump(BackfillPumpArgs),
    Status(BackfillStatusArgs),
    Failures(BackfillFailuresArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "pump")]
#[argh(description = "Attempt the next batch and report this pump's submission outcomes")]
struct BackfillPumpArgs {
    #[argh(positional)]
    #[argh(description = "stable string ID of the backfill")]
    backfill_id: String,

    #[argh(option)]
    #[argh(description = "maximum candidates to attempt in this pump (default: 100)")]
    limit: Option<u64>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "status")]
#[argh(description = "Show backfill task run counts by task type and status")]
struct BackfillStatusArgs {
    #[argh(positional)]
    #[argh(description = "stable string ID of the backfill")]
    backfill_id: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "failures")]
#[argh(description = "List terminally unsuccessful runs with the error each reported")]
struct BackfillFailuresArgs {
    #[argh(positional)]
    #[argh(description = "stable string ID of the backfill")]
    backfill_id: String,
}

pub async fn run(state: ApiCmdState, args: BackfillArgs) -> anyhow::Result<()> {
    match args.command {
        BackfillCommand::Pump(args) => {
            let request = backfill::BackfillPumpRequest {
                backfill_id: args.backfill_id,
                limit: args.limit,
            };
            let response = state.rest_client.admin_backfill_pump(&request).await?;
            println!("Backfill: {}", response.backfill_id);
            println!("Pump results (this invocation only):");
            println!("- Candidates attempted: {}", response.selected_count);
            println!("- Newly enqueued: {}", response.enqueued_count);
            println!(
                "- Skipped (already recorded for this backfill): {}",
                response.already_submitted_count
            );
            println!(
                "- Skipped (conflicting run already in flight): {}",
                response.refused_in_flight_count
            );
            println!("- Submission errors: {}", response.errors.len());
            for error in response.errors {
                println!("  - Entity {}: {}", error.entity_id, error.error);
            }
            println!(
                "For overall progress, use: backfill status {}",
                response.backfill_id
            );
        }
        BackfillCommand::Status(args) => {
            let response = state
                .rest_client
                .admin_backfill_status(&args.backfill_id)
                .await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        BackfillCommand::Failures(args) => {
            let response = state
                .rest_client
                .admin_backfill_failures(&args.backfill_id)
                .await?;
            println!("Backfill: {}", response.backfill_id);
            if response.failures.is_empty() {
                println!("No terminally unsuccessful runs.");
            } else {
                println!(
                    "Terminally unsuccessful runs (includes descendants): {}",
                    response.failures.len()
                );
                for failure in response.failures {
                    println!(
                        "- {} {} [{} run {}, {} attempts, {}]: {}",
                        failure.entity_type,
                        failure.entity_id,
                        failure.task_type,
                        failure.run_number,
                        failure.attempts,
                        failure.status.as_str(),
                        describe_error(failure.error_code, failure.error_message.as_deref()),
                    );
                }
            }
        }
    }

    Ok(())
}

/// One failure per line: an upstream message can arrive with embedded newlines.
fn describe_error(code: Option<i32>, message: Option<&str>) -> String {
    let message = message.map(|message| message.split_whitespace().collect::<Vec<_>>().join(" "));
    match (code, message) {
        (Some(code), Some(message)) if !message.is_empty() => format!("{code} {message}"),
        (Some(code), _) => code.to_string(),
        (None, Some(message)) if !message.is_empty() => message,
        (None, _) => "no error recorded".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::describe_error;

    #[test]
    fn failure_lines_carry_the_reported_code_and_a_flat_message() {
        assert_eq!(
            describe_error(Some(500), Some("vertex: quota\n  exceeded")),
            "500 vertex: quota exceeded"
        );
        assert_eq!(
            describe_error(Some(403), None),
            "403",
            "a code without a message is still reported"
        );
        assert_eq!(
            describe_error(None, None),
            "no error recorded",
            "rows written before schema version 7 have no error"
        );
    }
}
