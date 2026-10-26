use argh::FromArgs;

use crate::database;

#[derive(FromArgs)]
#[argh(subcommand, name = "first_user")]
#[argh(description = "Check if any users exist; if none, create the first admin user")]
pub struct FirstUserArgs {}

pub async fn run(db: &database::DbHandle, _args: FirstUserArgs) -> anyhow::Result<()> {
    database::check_first_user(db).await?;

    Ok(())
}
