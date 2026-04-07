use argh::FromArgs;

use crate::facility;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "first_user")]
#[argh(description = "Check if any users exist; if none, create the first admin user")]

pub struct FirstUserArgs {}

pub async fn run(state: CmdState, _args: FirstUserArgs) -> anyhow::Result<()> {
    facility::check_first_user(&state.db).await?;

    Ok(())
}
