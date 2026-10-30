use argh::FromArgs;
use base64::Engine;

use dreamscroll::{config, rest, telemetry, util_cmds};

/// Supply a JWT here to skip the OS keychain entirely: no read, no write, so a
/// non-interactive run never triggers a keychain permission prompt. Get one from
/// `POST /api/token`.
const TOKEN_ENV: &str = "DREAMSCROLL_API_TOKEN";

#[derive(FromArgs)]
#[argh(
    description = "dreamscroll REST API utility. Set DREAMSCROLL_API_TOKEN to authenticate \
                   without touching the keychain."
)]
struct Args {
    #[argh(
        option,
        long = "host",
        description = "REST API host (for example localhost:<PORT> or dreamscroll.ai)"
    )]
    host: String,

    #[argh(
        option,
        long = "user",
        description = "username for API auth; ignored when DREAMSCROLL_API_TOKEN is set"
    )]
    user: Option<String>,

    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Backfill(util_cmds::api::backfill::BackfillArgs),
    Captures(util_cmds::api::captures::CapturesArgs),
    ChangePassword(util_cmds::api::change_password::ChangePasswordArgs),
    ClearToken(util_cmds::api::clear_token::ClearTokenArgs),
    ExportDigest(util_cmds::api::export_digest::ExportDigestArgs),
    ImportDigest(util_cmds::api::import_digest::ImportDigestArgs),
    Search(util_cmds::api::search::SearchArgs),
    SearchSimilar(util_cmds::api::search_similar::SearchSimilarArgs),
    Spark(util_cmds::api::spark::SparkArgs),
    Illuminate(util_cmds::api::illuminate::IlluminateArgs),
    Inference(util_cmds::api::throughput::InferenceArgs),
    Queue(util_cmds::api::throughput::QueueArgs),
    UnknownEntities(util_cmds::api::entities::UnknownEntitiesArgs),
    Entity(util_cmds::api::entities::GetEntityArgs),
    ExportEntities(util_cmds::api::entities::ExportEntitiesArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let cfg = config::make()?;
    let args: Args = argh::from_env();
    let env_token = env_token();
    warn_on_redundant_user(env_token.as_deref(), args.user.as_deref());

    let username = match args.user.clone() {
        Some(username) => username,
        None => match &env_token {
            // Display only; the token already identifies the caller.
            Some(token) => token_username(token).unwrap_or_else(|| "token".to_string()),
            None => util_cmds::prompt_username_stdin()?,
        },
    };

    let client = match try_client_with_env_token(&args.host, env_token.as_deref()).await? {
        Some(client) => client,
        None => match try_client_from_token(&args.host, &username).await? {
            Some(client) => client,
            None => try_client_from_prompt(&args.host, &username).await?,
        },
    };
    let state = util_cmds::api::ApiCmdState::from_config(cfg, args.host, Some(username), client);

    match args.command {
        Command::Backfill(args) => util_cmds::api::backfill::run(state, args).await,
        Command::Captures(args) => util_cmds::api::captures::run(state, args).await,
        Command::ChangePassword(args) => util_cmds::api::change_password::run(state, args).await,
        Command::ClearToken(args) => util_cmds::api::clear_token::run(state, args).await,
        Command::ExportDigest(args) => util_cmds::api::export_digest::run(state, args).await,
        Command::ImportDigest(args) => util_cmds::api::import_digest::run(state, args).await,
        Command::Spark(args) => util_cmds::api::spark::run(state, args).await,
        Command::Search(args) => util_cmds::api::search::run(state, args).await,
        Command::SearchSimilar(args) => util_cmds::api::search_similar::run(state, args).await,
        Command::Illuminate(args) => util_cmds::api::illuminate::run(state, args).await,
        Command::Inference(args) => util_cmds::api::throughput::run_inference(state, args).await,
        Command::Queue(args) => util_cmds::api::throughput::run_queue(state, args).await,
        Command::UnknownEntities(args) => util_cmds::api::entities::run(state, args).await,
        Command::Entity(args) => util_cmds::api::entities::get(state, args).await,
        Command::ExportEntities(args) => util_cmds::api::entities::export(state, args).await,
    }
}

/// `--user` does not select the account when the token is supplied; say so rather
/// than letting an operator believe it did.
fn warn_on_redundant_user(token: Option<&str>, user: Option<&str>) {
    let (Some(token), Some(user)) = (token, user) else {
        return;
    };

    match token_username(token) {
        Some(caller) if caller != user => eprintln!(
            "warning: --user '{user}' is ignored for authentication; the token belongs to '{caller}'"
        ),
        Some(caller) => eprintln!(
            "warning: --user '{caller}' is redundant; DREAMSCROLL_API_TOKEN already identifies the caller"
        ),
        None => eprintln!(
            "warning: --user '{user}' is ignored for authentication; DREAMSCROLL_API_TOKEN is used instead"
        ),
    }
}

fn env_token() -> Option<String> {
    std::env::var(TOKEN_ENV)
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

/// Reads the `username` claim for display. Nothing is verified here — the server
/// validates the token on the first request.
fn token_username(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("username")?.as_str().map(str::to_string)
}

/// Uses `DREAMSCROLL_API_TOKEN` when present, without falling back to the
/// keychain or a password prompt: a rejected token is an error, not an
/// invitation to authenticate interactively.
async fn try_client_with_env_token(
    host: &str,
    token: Option<&str>,
) -> anyhow::Result<Option<rest::client::Client>> {
    let Some(token) = token else {
        return Ok(None);
    };

    let client = rest::client::Client::connect_with_token(host, token.to_string())?;
    client.validate_auth().await.map_err(|err| {
        anyhow::anyhow!(
            "{TOKEN_ENV} was rejected by '{host}': {err}. Unset it to log in interactively."
        )
    })?;

    Ok(Some(client))
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
