use argh::FromArgs;

use dreamscroll::{config, database, telemetry, util_cmds};

#[derive(FromArgs)]
#[argh(description = "dreamscroll direct database administration utility")]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    CreateUser(util_cmds::admin::create_user::CreateUserArgs),
    Enums(util_cmds::admin::enums::EnumsArgs),
    FirstUser(util_cmds::admin::first_user::FirstUserArgs),
    HashPassword(util_cmds::admin::hash_password::HashPasswordArgs),
    SyncSchema(util_cmds::admin::sync_schema::SyncSchemaArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let args: Args = argh::from_env();

    match args.command {
        Command::Enums(args) => util_cmds::admin::enums::run(args).await,
        Command::HashPassword(args) => util_cmds::admin::hash_password::run(args).await,
        command => run_database_command(command).await,
    }
}

async fn run_database_command(command: Command) -> anyhow::Result<()> {
    let postgres_config = config::make_postgres()?;
    let connection = database::connect_postgres_db(&postgres_config).await?;
    let db = database::DbHandle::new(connection);

    match command {
        Command::CreateUser(args) => util_cmds::admin::create_user::run(&db, args).await,
        Command::FirstUser(args) => util_cmds::admin::first_user::run(&db, args).await,
        Command::SyncSchema(args) => util_cmds::admin::sync_schema::run(&db, args).await,
        Command::Enums(_) | Command::HashPassword(_) => {
            unreachable!("non-database commands are dispatched before database setup")
        }
    }
}
