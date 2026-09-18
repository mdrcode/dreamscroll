use argh::FromArgs;

use dreamscroll::{config, telemetry, util_cmds};

#[derive(FromArgs)]
#[argh(description = "dreamscroll cmd line utility")]
struct Args {
    #[argh(
        option,
        long = "host",
        description = "REST API host override (default: localhost:<PORT from config>)"
    )]
    host: Option<String>,

    #[argh(
        switch,
        long = "prod",
        description = "convenience shortcut for --host dreamscroll.ai"
    )]
    prod: bool,

    #[argh(option, long = "user", description = "username for API auth")]
    user: Option<String>,

    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Backfill(util_cmds::backfill::BackfillArgs),
    ChangePassword(util_cmds::change_password::ChangePasswordArgs),
    CheckFirstUser(util_cmds::check_first_user::CheckFirstUserArgs),
    ClearToken(util_cmds::clear_token::ClearTokenArgs),
    CreateUser(util_cmds::create_user::CreateUserArgs),
    Enums(util_cmds::enums::EnumsArgs),
    ExportDigest(util_cmds::export_digest::ExportDigestArgs),
    FirstUser(util_cmds::first_user::FirstUserArgs),
    HashPassword(util_cmds::hash_password::HashPasswordArgs),
    IlluminateId(util_cmds::illuminate_id::IlluminateIdArgs),
    IlluminationText(util_cmds::illumination_text::IlluminationTextArgs),
    ImportDigest(util_cmds::import_digest::ImportDigestArgs),
    Search(util_cmds::search::SearchArgs),
    SearchIndex(util_cmds::search_index::SearchIndexArgs),
    SearchSimilar(util_cmds::search_similar::SearchSimilarArgs),
    Spark(util_cmds::spark::SparkArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();

    telemetry::init_local();

    let cfg = config::make()?;

    let args: Args = argh::from_env();

    let Args {
        host,
        prod,
        user,
        command,
    } = args;

    let rest_host = if let Some(host) = host {
        host
    } else if prod {
        "dreamscroll.ai".to_string()
    } else {
        format!("localhost:{}", cfg.port)
    };

    let state = util_cmds::CmdState::from_config(cfg, Some(rest_host), user).await?;

    match command {
        Command::Backfill(args) => util_cmds::backfill::run(state, args).await,
        Command::ChangePassword(args) => util_cmds::change_password::run(state, args).await,
        Command::CheckFirstUser(args) => util_cmds::check_first_user::run(state, args).await,
        Command::ClearToken(args) => util_cmds::clear_token::run(state, args).await,
        Command::CreateUser(args) => util_cmds::create_user::run(state, args).await,
        Command::Enums(args) => util_cmds::enums::run(state, args).await,
        Command::ExportDigest(args) => util_cmds::export_digest::run(state, args).await,
        Command::FirstUser(args) => util_cmds::first_user::run(state, args).await,
        Command::HashPassword(args) => util_cmds::hash_password::run(state, args).await,
        Command::IlluminateId(args) => util_cmds::illuminate_id::run(state, args).await,
        Command::IlluminationText(args) => util_cmds::illumination_text::run(state, args).await,
        Command::ImportDigest(args) => util_cmds::import_digest::run(state, args).await,
        Command::Search(args) => util_cmds::search::run(state, args).await,
        Command::SearchIndex(args) => util_cmds::search_index::run(state, args).await,
        Command::SearchSimilar(args) => util_cmds::search_similar::run(state, args).await,
        Command::Spark(args) => util_cmds::spark::run(state, args).await,
    }
}
