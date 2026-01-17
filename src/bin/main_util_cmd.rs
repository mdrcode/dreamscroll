use argh::FromArgs;

use dreamscroll::{facility, util_cmd::*};

#[derive(FromArgs)]
#[argh(description = "dreamscroll admin utility")]
struct Args {
    #[argh(subcommand)]
    command: Command,

    #[argh(
        switch,
        long = "verbose",
        short = 'v',
        description = "enable verbose logging"
    )]
    verbose: bool,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    CreateUser(create_user::CreateUserArgs),
    Illuminate(illuminate::IlluminateArgs),
    Import(import::ImportArgs),
    ExportUniq(export_uniq::ExportUniqArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    let mut config = facility::make_config(facility::Env::LocalDev);

    if args.verbose {
        config.tracing_max_level = tracing::Level::DEBUG;
    }

    facility::init_tracing(&config);

    match args.command {
        Command::CreateUser(args) => create_user::run(config, args).await,
        Command::Illuminate(args) => illuminate::run(config, args).await,
        Command::Import(args) => import::run(config, args).await,
        Command::ExportUniq(args) => export_uniq::run(config, args).await,
    }
}
