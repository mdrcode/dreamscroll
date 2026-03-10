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

    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    BackfillSearch(backfill_search::BackfillSearchArgs),
    CheckFirstUser(check_first_user::CheckFirstUserArgs),
    CreateUser(create_user::CreateUserArgs),
    Enums(enums::EnumsArgs),
    //Eval(eval::EvalArgs),
    ExportDigest(export_digest::ExportDigestArgs),
    FirstUser(first_user::FirstUserArgs),
    IlluminateAll(illuminate_all::IlluminateAllArgs),
    IlluminateId(illuminate_id::IlluminateIdArgs),
    ImportDigest(import_digest::ImportDigestArgs),
    Spark(spark::SparkArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Containerized environments should set NO_LOCAL_CONFIG_FILES=1 to skip
    // local config files. But we load them when running via `cargo run`
    if std::env::var("NO_LOCAL_CONFIG_FILES").is_err() {
        facility::load_local_config_files();
    }

    let args: Args = argh::from_env();

    facility::init_tracing().await?;
    let config = facility::make_config()?;

    let (db_connection, _) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::from_config(&config);

    // We use an empty beacon for the util commands, so no background tasks
    // will be enqueued.
    let empty_beacon = task::Beacon::default();
    let user_api =
        api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone(), empty_beacon);
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());

    let rest_host = args
        .host
        .clone()
        .unwrap_or_else(|| format!("localhost:{}", config.port));
    println!("Using REST host: {}", rest_host);

    let (username, password) = prompt_credentials_stdin()?;
    let rest_client = rest::client::Client::connect(&rest_host, &username, &password).await?;

    let state = CmdState {
        config,
        user_api,
        service_api,
        rest_client,
        rest_host,
        db,
        stg: stg,
    };

    match args.command {
        Command::BackfillSearch(args) => backfill_search::run(state, args).await,
        Command::CheckFirstUser(args) => check_first_user::run(state, args).await,
        Command::CreateUser(args) => create_user::run(state, args).await,
        Command::Enums(args) => enums::run(state, args).await,
        //Command::Eval(args) => eval::run(state, args).await,
        Command::ExportDigest(args) => export_digest::run(state, args).await,
        Command::FirstUser(args) => first_user::run(state, args).await,
        Command::IlluminateAll(args) => illuminate_all::run(state, args).await,
        Command::IlluminateId(args) => illuminate_id::run(state, args).await,
        Command::ImportDigest(args) => import_digest::run(state, args).await,
        Command::Spark(args) => spark::run(state, args).await,
    }
}
