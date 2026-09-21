use argh::FromArgs;

use dreamscroll::{config, telemetry, util_cmds};

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let cfg = config::make()?;
    let args: Args = argh::from_env();
    let state = util_cmds::admin::AdminCmdState::from_config(cfg).await?;

    match args.command {
        Command::CreateUser(args) => util_cmds::admin::create_user::run(state, args).await,
        Command::Enums(args) => util_cmds::admin::enums::run(state, args).await,
        Command::FirstUser(args) => util_cmds::admin::first_user::run(state, args).await,
        Command::HashPassword(args) => util_cmds::admin::hash_password::run(state, args).await,
    }
}
