use argh::FromArgs;

use crate::database;

#[derive(FromArgs)]
#[argh(subcommand, name = "sync_schema")]
#[argh(description = "Synchronize the application database schema and manual indexes")]
pub struct SyncSchemaArgs {}

pub async fn run(db: &database::DbHandle, _args: SyncSchemaArgs) -> anyhow::Result<()> {
    database::sync_schema(&db.conn).await?;
    println!("Database schema synchronized");
    Ok(())
}
