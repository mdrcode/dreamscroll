use anyhow::anyhow;
use argh::FromArgs;

use crate::api;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "backfill")]
#[argh(description = "Backfill task queue entries via admin API")]
pub struct BackfillArgs {
    #[argh(subcommand)]
    command: BackfillCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum BackfillCommand {
    SearchIndex(BackfillSearchIndexArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "search_index")]
#[argh(description = "Enqueue search index tasks")]
struct BackfillSearchIndexArgs {
    #[argh(positional)]
    #[argh(description = "capture IDs (omit when using --all)")]
    capture_ids: Vec<i32>,

    #[argh(switch)]
    #[argh(description = "target all captures that need search indexing")]
    all: bool,

    #[argh(option)]
    #[argh(description = "limit number of captures when using --all")]
    limit: Option<u64>,

    #[argh(switch)]
    #[argh(description = "required with --all when no --limit is provided")]
    force_all: bool,

    #[argh(switch)]
    #[argh(description = "preview candidate counts without enqueueing")]
    dry_run: bool,
}

pub async fn run(state: CmdState, args: BackfillArgs) -> anyhow::Result<()> {
    match args.command {
        BackfillCommand::SearchIndex(args) => run_search_index(state, args).await,
    }
}

async fn run_search_index(state: CmdState, args: BackfillSearchIndexArgs) -> anyhow::Result<()> {
    if args.all && !args.capture_ids.is_empty() {
        return Err(anyhow!(
            "Provide either --all or explicit capture IDs, not both"
        ));
    }

    if !args.all && args.capture_ids.is_empty() {
        return Err(anyhow!(
            "Provide explicit capture IDs or use --all for backfill"
        ));
    }

    if args.force_all && !args.all {
        return Err(anyhow!("--force-all requires --all"));
    }

    if args.all && !args.force_all && args.limit.is_none() {
        return Err(anyhow!("--all requires either --limit <N> or --force-all"));
    }

    let request = api::BackfillRequest {
        backfill_type: api::BackfillType::SearchIndex,
        all: args.all,
        force_all: args.force_all,
        limit: args.limit,
        capture_ids: if args.all {
            None
        } else {
            Some(args.capture_ids.clone())
        },
        dry_run: args.dry_run,
    };

    let rest_client = state.rest_client().await?;
    let response = rest_client.admin_enqueue_backfill(&request).await?;

    println!("Backfill enqueue response");
    println!("- Task type: search_index");
    println!("- Mode: {}", response.mode);
    println!("- Candidates: {}", response.candidate_count);
    println!("- Enqueued: {}", response.enqueued_count);
    println!("- Skipped: {}", response.skipped_count);

    if !response.skipped_ids.is_empty() {
        println!(
            "- Skipped IDs: {}",
            response
                .skipped_ids
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}
