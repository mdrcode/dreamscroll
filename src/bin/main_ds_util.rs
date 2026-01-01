use argh::FromArgs;

use dreamspot::util_cmd::*;

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

    let level = if args.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::WARN
    };
    tracing_subscriber::fmt().with_max_level(level).init();

    match args.command {
        Command::Illuminate(args) => illuminate::run(args).await?,
        Command::Import(args) => import::run(args).await?,
        Command::ExportUniq(args) => export_uniq::run(args).await?,
    }

    Ok(())
}
