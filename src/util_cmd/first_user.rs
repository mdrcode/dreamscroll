use argh::FromArgs;

use crate::facility;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "check_first_user")]
#[argh(description = "Check and create the first user, who is admin, in the database")]

pub struct FirstUserArgs {}

pub async fn run(state: CmdState, _args: FirstUserArgs) -> anyhow::Result<()> {
    facility::check_first_user(&state.db).await?;
    Ok(())
}
