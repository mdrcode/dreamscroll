use argh::FromArgs;

use dreamscroll::rest;
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
    IlluminationText(util_cmds::api::illumination_text::IlluminationTextArgs),
    ImportDigest(util_cmds::api::import_digest::ImportDigestArgs),
    Search(util_cmds::api::search::SearchArgs),
    SearchSimilar(util_cmds::api::search_similar::SearchSimilarArgs),
    Spark(util_cmds::api::spark::SparkArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let cfg = config::make()?;
    let args: Args = argh::from_env();
    let username = match args.user.clone() {
        Some(username) => username,
        None => util_cmds::prompt_username_stdin()?,
    };

    let client = match try_client_from_token(&args.host, &username).await? {
        Some(client) => client,
        None => try_client_from_prompt(&args.host, &username).await?,
    };
    let state = util_cmds::api::ApiCmdState::from_config(cfg, args.host, Some(username), client);

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
        Command::Search(args) => util_cmds::api::search::run(state, args).await,
        Command::SearchSimilar(args) => util_cmds::api::search_similar::run(state, args).await,
    }
}

async fn try_client_from_token(
    host: &str,
    username: &str,
) -> anyhow::Result<Option<rest::client::Client>> {
    if let Some(token) = util_cmds::api::token_cache::get_token(host, username)? {
        let client = rest::client::Client::connect_with_token(host, token)?;
        match client.validate_auth().await {
            Ok(()) => return Ok(Some(client)),
            Err(err) if err.to_string().contains("unauthorized (401)") => {
                let _ = util_cmds::api::token_cache::delete_token(host, username);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(None)
}

async fn try_client_from_prompt(
    host: &str,
    username: &str,
) -> anyhow::Result<rest::client::Client> {
    let password = util_cmds::prompt_password_stdin()?;
    let client = rest::client::Client::connect(host, username, &password).await?;

    if let Err(err) = util_cmds::api::token_cache::set_token(host, username, client.access_token())
    {
        eprintln!("Warning: unable to cache API token: {}", err);
    }

    Ok(client)
}
