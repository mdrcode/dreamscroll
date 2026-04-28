use anyhow::{Context, anyhow};
use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "illumination_text")]
#[argh(description = "Fetch illumination text via REST and print make_text() output")]
pub struct IlluminationTextArgs {
    #[argh(positional)]
    #[argh(description = "capture ID")]
    capture_id: i32,
}

pub async fn run(state: CmdState, args: IlluminationTextArgs) -> anyhow::Result<()> {
    let capture_id = args.capture_id;
    let rest_client = state.rest_client().await?;
    let captures = rest_client
        .get_captures(Some(&[capture_id]))
        .await
        .with_context(|| format!("failed to fetch capture {} via REST", capture_id))?;

    let capture = captures
        .into_iter()
        .find(|c| c.id == capture_id)
        .ok_or_else(|| anyhow!("capture {} not found", capture_id))?;

    let illumination = capture
        .illuminations
        .iter()
        .max_by_key(|illum| illum.id)
        .cloned()
        .ok_or_else(|| anyhow!("capture {} has no illuminations", capture_id))?;

    println!("{}", illumination.make_text());
    Ok(())
}
