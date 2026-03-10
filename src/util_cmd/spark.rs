use anyhow::anyhow;
use argh::FromArgs;

use crate::ignition::{Firestarter, grok::GrokFirestarter};
use crate::rest;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "spark")]
#[argh(description = "Generate a spark from one or more capture IDs")]
pub struct SparkArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) of the captures to spark from")]
    ids: Vec<i32>,

    #[argh(
        option,
        long = "host",
        default = "String::from(\"localhost:8080\")",
        description = "REST API host (default: localhost:8080)"
    )]
    host: String,
}

pub async fn run(state: CmdState, args: SparkArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let (username, password) = auth_helper::prompt_credentials_stdin()?;
    let rest_client = rest::client::Client::connect(&args.host, &username, &password).await?;

    let captures = rest_client.get_captures(Some(&args.ids)).await?;

    if captures.is_empty() {
        return Err(anyhow!("No matching captures found for provided IDs."));
    }

    let api_key = state
        .config
        .xai_api_key
        .clone()
        .ok_or_else(|| anyhow!("XAI_API_KEY is not configured"))?;

    let capture_count = captures.len();

    let firestarter = GrokFirestarter::new(api_key);
    let spark = firestarter.spark(captures).await?;

    let requested_ids = args
        .ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    println!("Spark request");
    println!("- Host: {}", args.host);
    println!("- Requested capture IDs: {}", requested_ids);
    println!("- Captures matched: {}", capture_count);
    println!();

    println!("{}", spark);

    Ok(())
}
