use argh::FromArgs;

use dreamscroll::{api, database, facility, storage, task, util_cmd::*};

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
    BackfillSearch(backfill_search::BackfillSearchArgs),
    CreateUser(create_user::CreateUserArgs),
    Enums(enums::EnumsArgs),
    Eval(eval::EvalArgs),
    ExportDigest(export_digest::ExportDigestArgs),
    FirstUser(first_user::FirstUserArgs),
    Illuminate(illuminate::IlluminateArgs),
    ImportDigest(import_digest::ImportDigestArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::from_filename("ds_config_local.env").ok();
    let _ = dotenvy::from_filename(".env"); // gitignored for api keys

    let args: Args = argh::from_env();
    if args.verbose {
        // TODO fix this back up later
    };

    facility::init_tracing();
    let config = facility::make_config();

    let (db_connection, _) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);

    // We use an empty beacon for the util commands, so no background tasks
    // will be enqueued.
    let empty_beacon = task::Beacon::default();
    let user_api =
        api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone(), empty_beacon);
    let import_api = api::ImportApiClient::new(db.clone(), stg.clone(), url_maker.clone());

    let cmd_state = CmdState {
        user_api,
        import_api,
        db,
        stg: stg,
    };

    match args.command {
        Command::BackfillSearch(args) => backfill_search::run(cmd_state, args).await,
        Command::CreateUser(args) => create_user::run(cmd_state, args).await,
        Command::Enums(args) => enums::run(cmd_state, args).await,
        Command::Eval(args) => eval::run(cmd_state, args).await,
        Command::ExportDigest(args) => export_digest::run(cmd_state, args).await,
        Command::FirstUser(args) => first_user::run(cmd_state, args).await,
        Command::Illuminate(args) => illuminate::run(cmd_state, args).await,
        Command::ImportDigest(args) => import_digest::run(cmd_state, args).await,
    }
}
