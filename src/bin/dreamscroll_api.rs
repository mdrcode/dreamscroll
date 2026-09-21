use argh::FromArgs;

use dreamscroll::{config, telemetry, util_cmds};

#[derive(FromArgs)]
#[argh(description = "dreamscroll REST API utility")]
struct Args {
    #[argh(
        option,
        long = "host",
        description = "REST API host (for example localhost:<PORT> or dreamscroll.ai)"
    )]
    host: String,

    #[argh(option, long = "user", description = "username for API auth")]
    user: Option<String>,

    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Backfill(util_cmds::api::backfill::BackfillArgs),
    ChangePassword(util_cmds::api::change_password::ChangePasswordArgs),
    ClearToken(util_cmds::api::clear_token::ClearTokenArgs),
    ExportDigest(util_cmds::api::export_digest::ExportDigestArgs),
    ImportDigest(util_cmds::api::import_digest::ImportDigestArgs),
    IlluminationText(util_cmds::api::illumination_text::IlluminationTextArgs),
    Spark(util_cmds::api::spark::SparkArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let cfg = config::make()?;
    let args: Args = argh::from_env();
    let state = util_cmds::api::ApiCmdState::from_config(cfg, args.host, args.user);

    match args.command {
        Command::Backfill(args) => util_cmds::api::backfill::run(state, args).await,
        Command::ChangePassword(args) => util_cmds::api::change_password::run(state, args).await,
        Command::ClearToken(args) => util_cmds::api::clear_token::run(state, args).await,
        Command::ExportDigest(args) => util_cmds::api::export_digest::run(state, args).await,
        Command::ImportDigest(args) => util_cmds::api::import_digest::run(state, args).await,
        Command::IlluminationText(args) => {
            util_cmds::api::illumination_text::run(state, args).await
        }
        Command::Spark(args) => util_cmds::api::spark::run(state, args).await,
    }
}
