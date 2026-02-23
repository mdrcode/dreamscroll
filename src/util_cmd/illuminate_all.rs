use argh::FromArgs;

use crate::{illumination, webhook};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "illuminate_all")]
#[argh(
    description = "Illuminate all captures currently missing illumination. Might take some time."
)]
pub struct IlluminateAllArgs {}

pub async fn run(state: CmdState, _args: IlluminateAllArgs) -> anyhow::Result<()> {
    let illuminator = illumination::make_illuminator("geministructured", state.stg.clone());

    let capture_ids = state.service_api.get_captures_need_illum().await?;

    tracing::info!("Found {} captures needing illumination.", capture_ids.len());

    for id in capture_ids {
        webhook::r_wh_illuminate::execute(&state.service_api, &illuminator, id).await?
    }

    Ok(())
}
