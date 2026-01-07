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
    };
    facility::init_logging(&config);

    match args.command {
        Command::Illuminate(args) => illuminate::run(config, args).await?,
        Command::Import(args) => import::run(config, args).await?,
        Command::ExportUniq(args) => export_uniq::run(config, args).await?,
    }

    Ok(())
}
