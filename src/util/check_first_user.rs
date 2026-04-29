use argh::FromArgs;

use crate::facility;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "check_first_user")]
#[argh(description = "Create first admin user if no users exist in the database")]
pub struct CheckFirstUserArgs {}

pub async fn run(mut state: CmdState, _args: CheckFirstUserArgs) -> anyhow::Result<()> {
    let db = state.db_handle().await?;
    facility::check_first_user(&db).await?;
    Ok(())
}
