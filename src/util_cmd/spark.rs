use anyhow::anyhow;
use argh::FromArgs;
use std::io::Write;

use crate::ignition::{
    Firestarter, SparkResponse, gemini::GeminiFirestarter, grok::GrokFirestarter,
};

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

    #[argh(
        option,
        long = "firestarter",
        short = 'f',
        default = "String::from(\"grok\")",
        description = "firestarter provider (grok, gemini) [default: grok]"
    )]
    firestarter: String,
}

pub async fn run(state: CmdState, args: SparkArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let captures = state.rest_client.get_captures(Some(&args.ids)).await?;

    if captures.is_empty() {
        return Err(anyhow!("No matching captures found for provided IDs."));
    }

    let capture_count = captures.len();

    let model = args.firestarter.trim().to_lowercase();
    let firestarter: Box<dyn Firestarter> = match model.as_str() {
        "grok" => {
            let api_key = state
                .config
                .xai_api_key
                .clone()
                .ok_or_else(|| anyhow!("XAI_API_KEY is not configured"))?;
            Box::new(GrokFirestarter::new(api_key))
        }
        "gemini" => {
            let api_key = state
                .config
                .gemini_api_key
                .clone()
                .ok_or_else(|| anyhow!("GEMINI_API_KEY is not configured"))?;
            Box::new(GeminiFirestarter::new(api_key))
        }
        _ => {
            return Err(anyhow!(
                "Unknown firestarter provider '{}'. Expected one of: grok, gemini",
                args.firestarter
            ));
        }
    };

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let spinner_task = tokio::spawn(run_spinner(stop_rx));

    let spark_start = std::time::Instant::now();
    let spark_result = firestarter.spark(captures).await;
    let _ = stop_tx.send(());
    let _ = spinner_task.await;

    let spark_result = spark_result?;
    let spark = spark_result.spark;
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
    println!("- Firestarter: {}", firestarter.name());
    println!("- Spark duration: {:?}", spark_duration);
    println!(
        "- Provider usage: in={:?} out={:?} total={:?} duration_ms={}",
        spark_result.meta.input_tokens,
        spark_result.meta.output_tokens,
        spark_result.meta.total_tokens,
        spark_result.meta.duration_ms
    );
    println!(
        "- Provider grounding json:\n{}",
        spark_result
            .meta
            .provider_grounding_json
            .unwrap_or_default()
    );

    println!();

    print_spark(&spark);

    Ok(())
}

fn print_spark(spark: &SparkResponse) {
    if spark.clusters.is_empty() {
        println!("No clusters returned.");
        return;
    }

    for (cluster_idx, cluster) in spark.clusters.iter().enumerate() {
        println!("Cluster {}", cluster_idx + 1);
        println!("Title: {}", cluster.title);
        println!("Summary: {}", cluster.summary);

        let capture_ids = cluster
            .capture_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        println!("Capture IDs: {}", capture_ids);

        if cluster.recommended_links.is_empty() {
            println!("Recommendations: none");
        } else {
            println!("Recommendations:");
            for (idx, rec) in cluster.recommended_links.iter().enumerate() {
                println!("  {}. {}", idx + 1, rec.url);
                println!("     {}", rec.commentary);
            }
        }

        if cluster_idx + 1 < spark.clusters.len() {
            println!();
        }
    }
}
