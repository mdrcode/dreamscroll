use argh::FromArgs;
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

use dreamscroll::model;
use dreamscroll::{config, database, knowledge, stats, telemetry};

#[derive(FromArgs)]
#[argh(description = "offline knowledge-graph tooling: observations, knowledge entities, evidence")]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Link(LinkArgs),
    Stats(StatsArgs),
    Merge(MergeArgs),
    Reset(ResetArgs),
    Compare(CompareArgs),
    Identity(IdentityArgs),
    Eval(EvalArgs),
    Proximity(ProximityArgs),
    Verdicts(VerdictsArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
#[argh(description = "replay a decision over cached answers, with no transport")]
struct EvalArgs {
    #[argh(subcommand)]
    command: EvalCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum EvalCommand {
    Resolve(EvalResolveArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "resolve")]
#[argh(description = "what identity resolve would claim today, replayed from the answer cache")]
struct EvalResolveArgs {
    #[argh(switch)]
    #[argh(description = "list every claim, not just the counts")]
    verbose: bool,
    #[argh(option)]
    #[argh(description = "consider at most this many entities")]
    limit: Option<usize>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "identity")]
#[argh(description = "what the world says a thing is: identities, and the claims behind them")]
struct IdentityArgs {
    #[argh(subcommand)]
    command: IdentityCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum IdentityCommand {
    Resolve(ResolveArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "link")]
#[argh(
    description = "link every unlinked observation to a knowledge entity, creating one as needed"
)]
struct LinkArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "merge")]
#[argh(description = "fold one knowledge entity into another; refuses when they share a capture")]
struct MergeArgs {
    #[argh(positional, description = "knowledge entity that will be tombstoned")]
    loser: i32,
    #[argh(positional, description = "knowledge entity that survives")]
    winner: i32,
    #[argh(switch, short = 'y')]
    #[argh(description = "apply the merge (otherwise it is a dry run)")]
    yes: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "proximity")]
#[argh(description = "pairs of entities the graph considers close, and what was decided")]
struct ProximityArgs {
    #[argh(subcommand)]
    command: ProximityCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum ProximityCommand {
    Scan(ProximityScanArgs),
    Merge(ProximityMergeArgs),
    List(ProximityListArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "scan")]
#[argh(description = "record every pair that shares an identity; changes nothing else")]
struct ProximityScanArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "merge")]
#[argh(description = "fold the open proximities that look like one thing")]
struct ProximityMergeArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "list")]
#[argh(description = "show what is in the table, most likely first")]
struct ProximityListArgs {
    #[argh(option, default = "50", description = "maximum rows to show")]
    limit: usize,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "verdicts")]
#[argh(description = "a person's decisions, applied again to the rebuilt graph")]
struct VerdictsArgs {
    #[argh(subcommand)]
    command: VerdictsCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum VerdictsCommand {
    Replay(VerdictsReplayArgs),
    List(VerdictsListArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "list")]
#[argh(description = "the modification log: what was folded into what, and what decided it")]
struct VerdictsListArgs {
    #[argh(option, description = "only this generation")]
    generation: Option<i32>,
    #[argh(option, description = "only this cause: repair, proximity_auto, replayed")]
    cause: Option<String>,
    #[argh(option, default = "50", description = "maximum rows to show")]
    limit: usize,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "replay")]
#[argh(
    description = "apply recorded verdicts to the current graph; sides that no longer map are \
                   reported, not guessed"
)]
struct VerdictsReplayArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "reset")]
#[argh(
    description = "snapshot the entities, evidence and membership, then empty them; needs --yes"
)]
struct ResetArgs {
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
#[argh(description = "ask evidence providers about entities with no authority claim (cache first)")]
struct ResolveArgs {
    #[argh(option)]
    #[argh(description = "transport calls allowed in this run (default 200)")]
    max_requests: Option<usize>,
    #[argh(switch)]
    #[argh(
        description = "serve from the cache only; makes no transport calls. Claims carried by \
                                 cached answers are still written — this is not a rehearsal"
    )]
    offline: bool,
    #[argh(option)]
    #[argh(description = "consider at most this many entities")]
    limit: Option<usize>,
    #[argh(option)]
    #[argh(description = "resolve only this entity id")]
    entity: Option<i32>,
    #[argh(option, default = "1000")]
    #[argh(description = "minimum milliseconds between transport calls")]
    min_interval_ms: u64,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "stats")]
#[argh(description = "counts: observations, linked share, entities by status, duplicate rate")]
struct StatsArgs {}

/// `eval resolve`: replay the resolver over the answer cache, with no transport.
async fn eval_resolve(
    db: &database::DbHandle,
    args: EvalResolveArgs,
) -> anyhow::Result<()> {
    let report = knowledge::eval::replay(db, args.limit).await?;
    if args.verbose {
        for claim in &report.claims {
            println!("{:<12} {:<14} {}", claim.identity, claim.method, claim.name);
        }
        println!();
    }
    println!(
        "candidates {} · claimed {} · undecided {} · not cached {}",
        report.candidates, report.claimed, report.undecided, report.uncached
    );
    if !report.by_method.is_empty() {
        let methods: Vec<String> = report
            .by_method
            .iter()
            .map(|(method, count)| format!("{method} {count}"))
            .collect();
        println!("by method: {}", methods.join(" · "));
    }
    Ok(())
}

/// `identity resolve`: ask the authorities what a thing is, cache first.
async fn resolve_identity(
    db: &database::DbHandle,
    args: ResolveArgs,
) -> anyhow::Result<()> {
    // A budget of zero makes no transport calls, which reads like a successful run that resolved
    // nothing. That is a mode, not a number.
    if args.offline && args.max_requests.is_some() {
        anyhow::bail!("--offline and --max-requests are mutually exclusive");
    }
    if args.max_requests == Some(0) {
        anyhow::bail!("--max-requests 0 makes no transport calls; pass --offline to ask for that");
    }
    let max_requests = match (args.offline, args.max_requests) {
        (true, _) => 0,
        (false, Some(n)) => n,
        (false, None) => knowledge::resolve::DEFAULT_MAX_REQUESTS,
    };
    let options = knowledge::resolve::Options {
        limit: args.limit,
        max_requests,
        offline: args.offline,
        only: args.entity,
        min_interval: std::time::Duration::from_millis(args.min_interval_ms),
    };
    let engine = knowledge::resolve::ResolutionEngine::new(db, options)
        .with(Box::new(knowledge::resolve::wikidata::Wikidata::new()?));
    let report = engine.run().await?;

    if report.offline {
        println!("offline — cache only, no transport calls");
    } else {
        println!("budget {max_requests} requests");
    }
    println!(
        "candidates {} · planned {} · cache hits {}",
        report.candidates, report.planned, report.cache_hits
    );
    println!(
        "fetched {} · failed {} · claims {} · entities resolved {}",
        report.fetched, report.failed, report.claims, report.entities_resolved
    );
    if report.budget_exhausted {
        println!("budget exhausted at {max_requests} requests");
    }
    if let Some(seconds) = report.rate_limited_after {
        // WDQS 429: honour Retry-After rather than risk a temporary ban.
        println!(
            "rate limited by the provider — stopped; retry in {seconds}s \
             (or lower --min-interval-ms / --max-requests)"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    telemetry::init_local();
    let args: Args = argh::from_env();

    let postgres_config = config::make_postgres()?;
    let connection = database::connect_postgres_db(&postgres_config).await?;
    let db = database::DbHandle::new(connection);

    match args.command {
        Command::Link(_) => {
            let report = link_all_pending(&db).await?;
            println!(
                "considered {} · already linked {} · linked {} · created {}",
                report.considered,
                report.already_linked,
                report.attached,
                report.created
            );
        }
        Command::Merge(args) => {
            let store = knowledge::Knowledge::new(&db);
            let preview = store.merge_preview(args.loser, args.winner).await?;
            println!(
                "merge {} -> {} · observations {} · evidence {} · shared illuminations {}",
                args.loser,
                args.winner,
                preview.observations,
                preview.evidence,
                preview.shared_illuminations
            );
            if preview.shared_illuminations > 0 {
                println!(
                    "refused: both entities appear in the same illumination, and one extraction \
                     emitting both means two referents"
                );
                return Ok(());
            }
            if !args.yes {
                println!("dry run; pass --yes to apply");
                return Ok(());
            }
            let cause = knowledge::merge::Cause::Repair;
            match store.merge(args.loser, args.winner, cause).await? {
                knowledge::merge::Outcome::Merged => {
                    println!("merged; {} is now a tombstone", args.loser)
                }
                knowledge::merge::Outcome::AlreadyMerged => {
                    println!("already merged; nothing to do")
                }
                knowledge::merge::Outcome::RefusedSharedIllumination => {
                    println!("refused at merge time — the entities changed under us")
                }
            }
        }
        Command::Verdicts(args) => match args.command {
            VerdictsCommand::Replay(_) => {
                let store = knowledge::Knowledge::new(&db);
                let report = knowledge::verdicts::replay(&store).await?;
                println!(
                    "applied {} · already {} · gone {} · refused {}",
                    report.applied, report.already, report.gone, report.refused
                );
            }
            VerdictsCommand::List(args) => {
                let rows = stats::verdicts::verdict_list(
                    &db,
                    args.generation,
                    args.cause.as_deref(),
                    args.limit,
                )
                .await?;
                for row in rows {
                    println!(
                        "{:>4}  gen {:<3} {:<15} {}  \u{21c4}  {}",
                        row.id,
                        row.generation,
                        row.cause,
                        row.winner,
                        row.loser
                    );
                }
            }
        },
        Command::Proximity(args) => {
            let store = knowledge::Knowledge::new(&db);
            match args.command {
                ProximityCommand::Scan(_) => {
                    println!(
                        "raised {} proximities",
                        knowledge::proximity::scan(&store).await?
                    );
                }
                ProximityCommand::Merge(_) => {
                    let report = knowledge::proximity::merge(&store).await?;
                    println!(
                        "merged {} · refused {} · moot {} · undecided {}",
                        report.merged, report.refused, report.moot, report.undecided
                    );
                }
                ProximityCommand::List(args) => {
                    let rows = stats::proximities::proximity_list(&db, args.limit).await?;
                    for row in rows {
                        let score = row.score.map_or("-".to_string(), |s| format!("{s:.2}"));
                        let decided = if row.decided { "same" } else { "" };
                        println!(
                            "{:>5}  {decided:<5}{score:>5}  {:<16}  {:<15}  {}  \u{21c4}  {}  [{}]",
                            row.id, row.note, row.basis, row.holder, row.counterpart, row.identity
                        );
                    }
                }
            }
        }
        Command::Reset(args) => {
            if !args.yes {
                anyhow::bail!("reset empties the projected graph; pass --yes to confirm");
            }
            let report = knowledge::snapshot::reset(&db).await?;
            match &report.snapshot {
                Some(snapshot) => println!(
                    "snapshot {} · entities {} · identities {} · claims {} · proximities {} · members {}",
                    snapshot.suffix,
                    report.snapshot_entities,
                    report.snapshot_identities,
                    report.snapshot_claims,
                    report.snapshot_proximities,
                    report.snapshot_members
                ),
                None => println!("nothing to snapshot"),
            }
            println!(
                "unlinked {} observations · deleted {} claims, {} merges, {} proximities · \
                 deleted {} entities",
                report.unlinked, report.claims, report.merges, report.proximities, report.entities
            );
        }
        Command::Compare(args) => {
            let before = knowledge::snapshot::Snapshot {
                suffix: args.before.clone(),
            };
            let after = knowledge::snapshot::Snapshot {
                suffix: args.after.clone(),
            };
            let report = knowledge::snapshot::compare(&db, &before, &after).await?;

            println!("before {} · after {}", args.before, args.after);
            println!(
                "observations {:>7}  (only before {}, only after {})",
                report.observations, report.only_in_before, report.only_in_after
            );
            println!(
                "entities     {:>7}  before {} · after {}",
                report.entities_before.max(report.entities_after),
                report.entities_before,
                report.entities_after
            );
            println!("unchanged    {:>7}  groups", report.unchanged);
            println!(
                "split {} · merged {} · moved {} observations",
                report.split, report.merged, report.moved
            );
            for example in &report.examples {
                match example.kind {
                    knowledge::snapshot::Kind::Split => println!(
                        "  split  \"{}\" · {} members → {} groups",
                        example.label, example.members, example.into
                    ),
                    knowledge::snapshot::Kind::Merged => println!(
                        "  merge  \"{}\" · {} groups → {} members",
                        example.label, example.into, example.members
                    ),
                }
            }
            if report.identical() {
                println!("no differences");
            }
        }
        Command::Eval(args) => match args.command {
            EvalCommand::Resolve(args) => eval_resolve(&db, args).await?,
        },
        Command::Identity(args) => match args.command {
            IdentityCommand::Resolve(args) => resolve_identity(&db, args).await?,
        },
        Command::Stats(_) => {
            let stats = stats::knowledge_stats(&db).await?;
            println!("observations        {:>7}", stats.observations);
            println!("  linked            {:>7}", stats.linked);
            println!("knowledge entities  {:>7}", stats.entities);
            println!("  provisional       {:>7}", stats.provisional);
            println!("  identified        {:>7}", stats.identified);
            println!("duplicate rate      {:>7.3}", stats.duplicate_rate());
            println!("proximities");
            println!("  {:<18}{:>7}", "candidates", stats.proximities);
            println!(
                "  {:<18}{:>7}",
                "entities in pairs", stats.entities_in_proximity
            );
            println!("resolution");
            println!("  with authority claim {:>5}", stats.with_claim);
            println!(
                "  platform presence      {} projected · {} from members",
                stats.with_platform, stats.platform_from_members
            );
            println!(
                "  cache {} with candidates · {} no match · {} failed · {} degraded · {} expired",
                stats.cache_found,
                stats.cache_empty,
                stats.cache_failed,
                stats.cache_degraded,
                stats.cache_expired
            );
        }
    }

    Ok(())
}

/// Stats from a run of linking all pending observations into the knowledge graph.
///
/// Lives here rather than in the library because it describes a *run*, not a link decision: how many
/// illuminations there were and how the totals add up. The library reports what it wrote. When
/// link_all_pending is run after a reset, these stats reflect the entire rebuild of the knowledge graph.
#[derive(Debug, Default)]
struct LinkAllPendingReport {
    considered: usize,
    already_linked: usize,
    attached: usize,
    created: usize,
}

impl LinkAllPendingReport {
    fn update(&mut self, part: knowledge::LinkStats) {
        self.attached += part.attached;
        self.created += part.created;
    }
}

/// Link every illumination that still has unlinked observations.
///
/// After a reset, this rebuilds the entire knowledge graph.
async fn link_all_pending(db: &database::DbHandle) -> anyhow::Result<LinkAllPendingReport> {
    let unlinked_obs = Query::select()
        .column(model::user_observed_entity::Column::IlluminationId)
        .from(model::user_observed_entity::Entity)
        .and_where(model::user_observed_entity::Column::KnowledgeEntityId.is_null())
        .to_owned();

    let illuminations = model::illumination::Entity::load()
        .filter(Expr::col(model::illumination::Column::Id).in_subquery(unlinked_obs))
        .with(model::user_observed_entity::Entity)
        .order_by_asc(model::illumination::Column::Id)
        .all(&db.conn)
        .await?;

    let knowledge = knowledge::Knowledge::with_linking(
        db,
        Box::new(knowledge::heuristic::NameBlocker::new(db).await?),
        std::sync::Arc::new(knowledge::heuristic::HeuristicLinker),
    );
    let mut report = LinkAllPendingReport::default();
    for illumination in &illuminations {
        for observation in illumination.observations.iter() {
            report.considered += 1;
            if observation.knowledge_entity_id.is_some() {
                report.already_linked += 1;
            }
        }
        let applied = knowledge.link_illumination(illumination).await?;
        report.update(applied);
    }
    Ok(report)
}
