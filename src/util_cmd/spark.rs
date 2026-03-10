use anyhow::anyhow;
use argh::FromArgs;
use std::io::Write;

use crate::ignition::{Firestarter, grok::GrokFirestarter};

use super::*;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

async fn run_spinner(mut stop_rx: tokio::sync::oneshot::Receiver<()>) {
    let mut idx = 0usize;

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                print!("\r✓ Spark inference complete.                    \n");
                let _ = std::io::stdout().flush();
                break;
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(90)) => {
                let frame = SPINNER_FRAMES[idx % SPINNER_FRAMES.len()];
                print!("\r{} Sending spark for inference...", frame);
                let _ = std::io::stdout().flush();
                idx += 1;
            }
        }
    }
}

#[derive(FromArgs)]
#[argh(subcommand, name = "spark")]
#[argh(description = "Generate a spark from one or more capture IDs")]
pub struct SparkArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) of the captures to spark from")]
    ids: Vec<i32>,
}

pub async fn run(state: CmdState, args: SparkArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let captures = state.rest_client.get_captures(Some(&args.ids)).await?;

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
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let spinner_task = tokio::spawn(run_spinner(stop_rx));

    let spark_start = std::time::Instant::now();
    let spark_result = firestarter.spark(captures).await;
    let _ = stop_tx.send(());
    let _ = spinner_task.await;

    let spark = spark_result?;
    let spark_duration = spark_start.elapsed();

    let requested_ids = args
        .ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    println!("Spark request");
    println!("- Host: {}", state.rest_host);
    println!("- Requested capture IDs: {}", requested_ids);
    println!("- Captures matched: {}", capture_count);
    println!("- Spark duration: {:?}", spark_duration);

    println!();

    println!("{}", spark);

    Ok(())
}
