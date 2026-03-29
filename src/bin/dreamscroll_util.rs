use anyhow::Context;
use argh::FromArgs;

use dreamscroll::{api, database, facility, rest, storage, task, util_cmd::*};

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
    BackfillSearch(backfill_search::BackfillSearchArgs),
    CheckFirstUser(check_first_user::CheckFirstUserArgs),
    ClearToken(clear_token::ClearTokenArgs),
    CreateUser(create_user::CreateUserArgs),
    Enums(enums::EnumsArgs),
    //Eval(eval::EvalArgs),
    ExportDigest(export_digest::ExportDigestArgs),
    FirstUser(first_user::FirstUserArgs),
    IlluminateAll(illuminate_all::IlluminateAllArgs),
    IlluminateId(illuminate_id::IlluminateIdArgs),
    ImportDigest(import_digest::ImportDigestArgs),
    SearchEmbed(search_embed::SearchEmbedArgs),
    Spark(spark::SparkArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Containerized environments should set NO_LOCAL_CONFIG_FILES=1 to skip
    // local config files. But we load them when running via `cargo run`
    if std::env::var("NO_LOCAL_CONFIG_FILES").is_err() {
        facility::load_local_config_files();
    }

    facility::init_tracing().await?;
    let config = facility::make_config()?;

    let (db_connection, _) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::from_config(&config);

    // We use an empty beacon for the util commands, so no background tasks
    // will be enqueued.
    // TODO this should be a NOOP queue that logs tasks so we can verify behavior
    let empty_beacon = task::Beacon::default();
    let user_api =
        api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone(), empty_beacon);
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());

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
        format!("localhost:{}", config.port)
    };
    println!("Using REST host: {}", rest_host);

    let username = if let Some(user) = user {
        user.trim().to_string()
    } else {
        prompt_username_stdin()?
    };

    if username.is_empty() {
        anyhow::bail!("Username cannot be empty. Provide --user or enter a username.");
    }

    let command = match command {
        Command::ClearToken(clear_args) => {
            return clear_token::run(&rest_host, &username, clear_args).await;
        }
        other => other,
    };

    let rest_client = if let Some(cached_token) = token_cache::get_token(&rest_host, &username)? {
        println!(
            "Found cached API token for host='{}' username='{}'.",
            rest_host, username
        );
        let cached_client = rest::client::Client::connect_with_token(&rest_host, cached_token)
            .context("failed to initialize REST client from cached token")?;

        match cached_client.validate_auth().await {
            Ok(()) => {
                println!("Cached API token is valid.");
                cached_client
            }
            Err(err) => {
                if err.to_string().contains("unauthorized (401)") {
                    println!("Cached token expired or invalid; requesting a new token.");
                    let _ = token_cache::delete_token(&rest_host, &username);
                    let password = prompt_password_stdin()?;
                    let fresh_client =
                        rest::client::Client::connect(&rest_host, &username, &password).await?;
                    println!("Successfully authenticated and retrieved API token.");
                    if let Err(cache_err) =
                        token_cache::set_token(&rest_host, &username, fresh_client.access_token())
                    {
                        eprintln!("Warning: unable to cache API token: {}", cache_err);
                    } else {
                        println!("Successfully cached API token.");
                    }
                    fresh_client
                } else {
                    println!("Cached token validation failed for a non-auth reason.");
                    return Err(err).context("failed to validate cached API token");
                }
            }
        }
    } else {
        let password = prompt_password_stdin()?;
        let fresh_client = rest::client::Client::connect(&rest_host, &username, &password).await?;
        println!("Successfully authenticated and retrieved API token.");
        if let Err(cache_err) =
            token_cache::set_token(&rest_host, &username, fresh_client.access_token())
        {
            eprintln!("Warning: unable to cache API token: {}", cache_err);
        } else {
            println!("Successfully cached API token.");
        }
        fresh_client
    };

    let state = CmdState {
        config,
        user_api,
        service_api,
        rest_client,
        rest_host,
        db,
        stg: stg,
    };

    match command {
        Command::BackfillSearch(args) => backfill_search::run(state, args).await,
        Command::CheckFirstUser(args) => check_first_user::run(state, args).await,
        Command::ClearToken(_) => anyhow::bail!("clear_token should have exited earlier"),
        Command::CreateUser(args) => create_user::run(state, args).await,
        Command::Enums(args) => enums::run(state, args).await,
        //Command::Eval(args) => eval::run(state, args).await,
        Command::ExportDigest(args) => export_digest::run(state, args).await,
        Command::FirstUser(args) => first_user::run(state, args).await,
        Command::IlluminateAll(args) => illuminate_all::run(state, args).await,
        Command::IlluminateId(args) => illuminate_id::run(state, args).await,
        Command::ImportDigest(args) => import_digest::run(state, args).await,
        Command::SearchEmbed(args) => search_embed::run(state, args).await,
        Command::Spark(args) => spark::run(state, args).await,
    }
}
