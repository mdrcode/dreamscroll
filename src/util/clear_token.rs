use anyhow::Context;
use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "clear_token")]
#[argh(description = "Clear cached API token for the selected host and username")]
pub struct ClearTokenArgs {}

pub async fn run(state: CmdState, _args: ClearTokenArgs) -> anyhow::Result<()> {
    let rest_host = state.rest_host.as_deref().context("REST host missing")?;
    let rest_user = state
        .rest_user
        .as_deref()
        .context("REST username missing")?;

    token_cache::delete_token(rest_host, rest_user)?;

    println!("Cleared cached API token.");
    println!("- Host: {}", rest_host);
    println!("- Username: {}", rest_user);

    Ok(())
}
