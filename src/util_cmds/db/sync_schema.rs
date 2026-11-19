use argh::FromArgs;

use crate::database;

#[derive(FromArgs)]
#[argh(subcommand, name = "sync_schema")]
#[argh(description = "Synchronize the application database schema and manual indexes")]
pub struct SyncSchemaArgs {
    #[argh(switch)]
    #[argh(
        description = "run the whole reconciliation in a transaction and roll it back; reports \
                       whether the release's sync would succeed without changing anything"
    )]
    pub dry_run: bool,
}

pub async fn run(db: &database::DbHandle, args: SyncSchemaArgs) -> anyhow::Result<()> {
    if args.dry_run {
        database::sync_schema_dry_run(&db.conn).await?;
        println!("Schema dry run succeeded; rolled back");
    } else {
        database::sync_schema(&db.conn).await?;
        println!("Database schema synchronized");
    }
    Ok(())
}
