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
    }

    Ok(())
}
