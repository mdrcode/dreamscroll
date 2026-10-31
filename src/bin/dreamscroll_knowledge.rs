use argh::FromArgs;

use dreamscroll::{config, database, knowledge, telemetry};

#[derive(FromArgs)]
#[argh(description = "offline knowledge-graph tooling: observations, cores, evidence")]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Migrate(MigrateArgs),
    BuildCore(BuildCoreArgs),
    Stats(StatsArgs),
    ResetCore(ResetCoreArgs),
    Compare(CompareArgs),
    Resolve(ResolveArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "migrate")]
#[argh(description = "copy observations from knowledge_user into user_knowledge_entities (idempotent)")]
struct MigrateArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "build_core")]
#[argh(description = "link every unlinked observation to a core, creating cores as needed")]
struct BuildCoreArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "reset_core")]
#[argh(description = "snapshot the cores, evidence and membership, then empty them; needs --yes")]
struct ResetCoreArgs {
    #[argh(switch, short = 'y')]
    #[argh(description = "confirm resetting the projected graph")]
    yes: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "compare")]
#[argh(description = "diff two snapshots' membership, by observation id")]
struct CompareArgs {
    #[argh(positional, description = "suffix of the earlier snapshot")]
    before: String,
    #[argh(positional, description = "suffix of the later snapshot")]
    after: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "resolve")]
#[argh(description = "ask evidence providers about cores with no authority claim (cache first)")]
struct ResolveArgs {
    #[argh(switch)]
    #[argh(description = "plan and report without touching the network")]
    dry_run: bool,
    #[argh(option, default = "200")]
    #[argh(description = "transport calls allowed in this run")]
    max_requests: usize,
    #[argh(option)]
    #[argh(description = "consider at most this many cores")]
    limit: Option<usize>,
    #[argh(option)]
    #[argh(description = "resolve only this core id")]
    core: Option<i32>,
    #[argh(option, default = "1000")]
    #[argh(description = "minimum milliseconds between transport calls")]
    min_interval_ms: u64,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "stats")]
#[argh(description = "counts: observations, linked share, cores by status, duplicate rate")]
struct StatsArgs {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let args: Args = argh::from_env();

    let postgres_config = config::make_postgres()?;
    let connection = database::connect_postgres_db(&postgres_config).await?;
    let db = database::DbHandle::new(connection);

    match args.command {
        Command::Migrate(_) => {
            let report = knowledge::migrate::migrate(&db).await?;
            println!(
                "source rows {} · already present {} · copied {}",
                report.source, report.already_present, report.copied
            );
        }
        Command::BuildCore(_) => {
            let report = knowledge::link::build_core(&db).await?;
            println!(
                "considered {} · already linked {} · superseded {} · linked {} · created {}",
                report.considered,
                report.already_linked,
                report.superseded,
                report.linked,
                report.created
            );
        }
        Command::ResetCore(args) => {
            if !args.yes {
                anyhow::bail!(
                    "reset_core empties the projected graph; pass --yes to confirm"
                );
            }
            let report = knowledge::reset::reset_core(&db).await?;
            match &report.snapshot {
                Some(snapshot) => println!(
                    "snapshot {} · cores {} · evidence {} · members {}",
                    snapshot.suffix,
                    report.snapshot_cores,
                    report.snapshot_evidence,
                    report.snapshot_members
                ),
                None => println!("nothing to snapshot"),
            }
            println!(
                "unlinked {} observations · deleted {} evidence rows · deleted {} cores",
                report.unlinked, report.evidence, report.cores
            );
        }
        Command::Compare(args) => {
            let before = knowledge::reset::Snapshot {
                suffix: args.before.clone(),
            };
            let after = knowledge::reset::Snapshot {
                suffix: args.after.clone(),
            };
            let report = knowledge::compare::compare(&db, &before, &after).await?;

            println!("before {} · after {}", args.before, args.after);
            println!(
                "observations {:>7}  (only before {}, only after {})",
                report.observations, report.only_in_before, report.only_in_after
            );
            println!(
                "cores        {:>7}  before {} · after {}",
                report.cores_before.max(report.cores_after),
                report.cores_before,
                report.cores_after
            );
            println!("unchanged    {:>7}  groups", report.unchanged);
            println!(
                "split {} · merged {} · moved {} observations",
                report.split, report.merged, report.moved
            );
            for example in &report.examples {
                match example.kind {
                    knowledge::compare::Kind::Split => println!(
                        "  split  \"{}\" · {} members → {} groups",
                        example.label, example.members, example.into
                    ),
                    knowledge::compare::Kind::Merged => println!(
                        "  merge  \"{}\" · {} groups → {} members",
                        example.label, example.into, example.members
                    ),
                }
            }
            if report.identical() {
                println!("no differences");
            }
        }
        Command::Resolve(args) => {
            let engines: Vec<Box<dyn knowledge::resolve::Engine>> = vec![Box::new(
                knowledge::resolve::wikidata::Wikidata::new()?,
            )];
            let options = knowledge::resolve::Options {
                limit: args.limit,
                max_requests: args.max_requests,
                dry_run: args.dry_run,
                only: args.core,
                min_interval: std::time::Duration::from_millis(args.min_interval_ms),
            };
            let report = knowledge::resolve::resolve(&db, &engines, &options).await?;

            if report.dry_run {
                println!("dry run — nothing fetched");
            }
            println!(
                "candidates {} · planned {} · cache hits {}",
                report.candidates, report.planned, report.cache_hits
            );
            println!(
                "fetched {} · failed {} · claims {} · cores resolved {} · conflicts {}",
                report.fetched, report.failed, report.claims, report.cores_resolved, report.conflicts
            );
            if report.budget_exhausted {
                println!("budget exhausted at {} requests", args.max_requests);
            }
            if let Some(seconds) = report.rate_limited_after {
                // WDQS 429: honour Retry-After rather than risk a temporary ban.
                println!(
                    "rate limited by the provider — stopped; retry in {seconds}s \
                     (or lower --min-interval-ms / --max-requests)"
                );
            }
        }
        Command::Stats(_) => {
            let stats = knowledge::stats::stats(&db).await?;
            println!("observations        {:>7}", stats.observations);
            println!("  linked            {:>7}", stats.linked);
            println!("cores               {:>7}", stats.cores);
            println!("  provisional       {:>7}", stats.provisional);
            println!("  identified        {:>7}", stats.identified);
            println!("  contested         {:>7}", stats.contested);
            println!("duplicate rate      {:>7.3}", stats.duplicate_rate());
            println!("resolution");
            println!("  with ontology claim {:>5}", stats.with_ontology);
            println!(
                "  cache {} with candidates · {} no match · {} failed · {} expired",
                stats.cache_found, stats.cache_empty, stats.cache_failed, stats.cache_expired
            );
        }
    }

    Ok(())
}
