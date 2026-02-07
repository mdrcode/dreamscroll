use argh::FromArgs;

use dreamscroll::{api, database, facility, storage, util_cmd::*};

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
    Eval(eval::EvalArgs),
    ExportUniq(export_uniq::ExportUniqArgs),
    ExportDigest(export_digest::ExportDigestArgs),
    Illuminate(illuminate::IlluminateArgs),
    Import(import::ImportArgs),
    ImportDigest(import_digest::ImportDigestArgs),
    Enums(enums::EnumsArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    let mut config = facility::make_config();

    if args.verbose {
        config.tracing_max_level = tracing::Level::DEBUG;
    }

    facility::init_tracing(&config);

    let (db_connection, _) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    let stg = storage::make_provider(config.storage.clone()).await;
    let url_maker = storage::UrlMaker::new(config.storage_url_maker.clone());
    let api_client = api::ApiClient::new(db.clone(), stg.clone(), url_maker);

    let cmd_state = CmdState {
        api_client,
        db,
        stg: stg,
    };

    match args.command {
        Command::BackfillSearch(args) => backfill_search::run(cmd_state, args).await,
        Command::Eval(args) => eval::run(cmd_state, args).await,
        Command::CreateUser(args) => create_user::run(cmd_state, args).await,
        Command::Illuminate(args) => illuminate::run(cmd_state, args).await,
        Command::Import(args) => import::run(cmd_state, args).await,
        Command::ImportDigest(args) => import_digest::run(cmd_state, args).await,
        Command::ExportUniq(args) => export_uniq::run(cmd_state, args).await,
        Command::ExportDigest(args) => export_digest::run(cmd_state, args).await,
        Command::Enums(args) => enums::run(cmd_state, args).await,
    }
}
