use argh::FromArgs;
use chrono::{DateTime, SecondsFormat, Utc};

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

    #[argh(switch)]
    #[argh(
        description = "pump despite a recorded dry state; required after the pump found nothing"
    )]
    force: bool,
    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "status")]
#[argh(description = "Show backfill task run counts by task type and status")]
struct BackfillStatusArgs {
    #[argh(positional)]
    #[argh(description = "stable string ID of the backfill")]
    backfill_id: String,
    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "failures")]
#[argh(description = "List terminally unsuccessful runs with the error each reported")]
struct BackfillFailuresArgs {
    #[argh(positional)]
    #[argh(description = "stable string ID of the backfill")]
    backfill_id: String,
    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

pub async fn run(state: ApiCmdState, args: BackfillArgs) -> anyhow::Result<()> {
    match args.command {
        BackfillCommand::Pump(args) => {
            let request = backfill::BackfillPumpRequest {
                backfill_id: args.backfill_id,
                limit: args.limit,
                force: args.force,
            };
            let response = match state.rest_client.admin_backfill_pump(&request).await {
                Ok(response) => response,
                // A dry pump is refused with 409 and an opaque body (status +
                // report_id); the status code is the contract.
                Err(error) if error.to_string().contains("409 Conflict") => anyhow::bail!(
                    "Pump refused: {} is dry. Run `backfill status {}` for the dry \
                     timestamp, or pass --force to pump again. ({error})",
                    request.backfill_id,
                    request.backfill_id,
                ),
                Err(error) => return Err(error),
            };
            if args.raw_json {
                return print_json(&response);
            }
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
            println!("- Recorded at: {}", format_time(response.pumped_at));
            match response.dry_since {
                Some(dry_since) => println!(
                    "- Dry since {}: no eligible targets; later pumps refuse unless --force is passed",
                    format_time(dry_since)
                ),
                None => println!("- Still open: this pump selected eligible targets"),
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
            if args.raw_json {
                return print_json(&response);
            }
            println!("Backfill: {}", response.backfill_id);
            println!(
                "Last pumped: {}",
                response
                    .last_pumped_at
                    .map(format_time)
                    .unwrap_or_else(|| "never".to_string())
            );
            match response.dry_since {
                Some(dry_since) => println!(
                    "Dry since {}: pumps refuse unless --force is passed.",
                    format_time(dry_since)
                ),
                None => println!("Still open: no recorded dry streak."),
            }
            if response.counts.is_empty() {
                println!("No tagged task runs yet.");
            } else {
                println!("Runs (includes descendants):");
                println!("  {:<20} {:<20} {:>6}", "task_type", "status", "count");
                for count in &response.counts {
                    println!(
                        "  {:<20} {:<20} {:>6}",
                        count.task_type,
                        count.status.as_str(),
                        count.count
                    );
                }
            }
        }
        BackfillCommand::Failures(args) => {
            let response = state
                .rest_client
                .admin_backfill_failures(&args.backfill_id)
                .await?;
            if args.raw_json {
                return print_json(&response);
            }
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

fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
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
