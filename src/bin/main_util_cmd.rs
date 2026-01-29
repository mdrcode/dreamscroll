use argh::FromArgs;

use dreamscroll::{facility, util_cmd::*};

#[derive(FromArgs)]
#[argh(description = "dreamscroll admin utility")]
struct Args {
    #[argh(subcommand)]
    command: Command,

    #[argh(switch, long = "verbose", description = "enable verbose logging")]
    verbose: bool,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    CreateUser(create_user::CreateUserArgs),
    Eval(eval::EvalArgs),
    ExportUniq(export_uniq::ExportUniqArgs),
    ExportWithDigest(export_with_digest::ExportWithDigestArgs),
    Illuminate(illuminate::IlluminateArgs),
    Import(import::ImportArgs),
    KNodes(knodes::KNodesArgs),
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
        Command::Eval(args) => eval::run(config, args).await,
        Command::CreateUser(args) => create_user::run(config, args).await,
        Command::Illuminate(args) => illuminate::run(config, args).await,
        Command::Import(args) => import::run(config, args).await,
        Command::ExportUniq(args) => export_uniq::run(config, args).await,
        Command::ExportWithDigest(args) => export_with_digest::run(config, args).await,
        Command::KNodes(args) => knodes::run(config, args).await,
    }
}
