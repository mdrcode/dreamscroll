use argh::FromArgs;

use dreamscroll::{config, telemetry, util};

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
    Backfill(util::backfill::BackfillArgs),
    ChangePassword(util::change_password::ChangePasswordArgs),
    CheckFirstUser(util::check_first_user::CheckFirstUserArgs),
    ClearToken(util::clear_token::ClearTokenArgs),
    CreateUser(util::create_user::CreateUserArgs),
    Enums(util::enums::EnumsArgs),
    ExportDigest(util::export_digest::ExportDigestArgs),
    FirstUser(util::first_user::FirstUserArgs),
    HashPassword(util::hash_password::HashPasswordArgs),
    IlluminateId(util::illuminate_id::IlluminateIdArgs),
    IlluminationText(util::illumination_text::IlluminationTextArgs),
    ImportDigest(util::import_digest::ImportDigestArgs),
    Search(util::search::SearchArgs),
    SearchIndex(util::search_index::SearchIndexArgs),
    SearchSimilar(util::search_similar::SearchSimilarArgs),
    Spark(util::spark::SparkArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::import_local_if_test_or_dev();

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

    let state = util::CmdState::from_config(cfg, Some(rest_host), user).await?;

    match command {
        Command::Backfill(args) => util::backfill::run(state, args).await,
        Command::ChangePassword(args) => util::change_password::run(state, args).await,
        Command::CheckFirstUser(args) => util::check_first_user::run(state, args).await,
        Command::ClearToken(args) => util::clear_token::run(state, args).await,
        Command::CreateUser(args) => util::create_user::run(state, args).await,
        Command::Enums(args) => util::enums::run(state, args).await,
        Command::ExportDigest(args) => util::export_digest::run(state, args).await,
        Command::FirstUser(args) => util::first_user::run(state, args).await,
        Command::HashPassword(args) => util::hash_password::run(state, args).await,
        Command::IlluminateId(args) => util::illuminate_id::run(state, args).await,
        Command::IlluminationText(args) => util::illumination_text::run(state, args).await,
        Command::ImportDigest(args) => util::import_digest::run(state, args).await,
        Command::Search(args) => util::search::run(state, args).await,
        Command::SearchIndex(args) => util::search_index::run(state, args).await,
        Command::SearchSimilar(args) => util::search_similar::run(state, args).await,
        Command::Spark(args) => util::spark::run(state, args).await,
    }
}
