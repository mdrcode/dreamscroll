use argh::FromArgs;

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
    Index(IndexArgs),
    Proximity(ProximityArgs),
    Verdicts(VerdictsArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "index")]
#[argh(description = "knowledge_blocking_index: the folded forms a blocker matches on")]
struct IndexArgs {
    #[argh(subcommand)]
    command: IndexCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum IndexCommand {
    Rebuild(IndexRebuildArgs),
    Check(IndexCheckArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "rebuild")]
#[argh(description = "recompute every index row from the entity columns")]
struct IndexRebuildArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "check")]
#[argh(description = "rows built with an older fold, and live entities with no row")]
struct IndexCheckArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
#[argh(description = "diagnose one entity's path, or measure blocking recall")]
struct EvalArgs {
    #[argh(subcommand)]
    command: EvalCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum EvalCommand {
    Inspect(EvalInspectArgs),
    Blocker(EvalBlockerArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "inspect")]
#[argh(description = "walk one entity through blocking, ranking and the resolver ladder")]
struct EvalInspectArgs {
    #[argh(
        positional,
        description = "entity id, or a name that folds to the same surface"
    )]
    target: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "blocker")]
#[argh(description = "did the blocker offer the entity each observation ended up in?")]
struct EvalBlockerArgs {}

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
    Reproject(ReprojectArgs),
    Veto(VetoArgs),
    Unveto(UnvetoArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "veto")]
#[argh(description = "an entity is not an identity: record it and withdraw the claim")]
struct VetoArgs {
    #[argh(option, description = "knowledge entity id")]
    entity: i32,
    #[argh(option, description = "the identity's external id, e.g. Q122707294")]
    identity: String,
    #[argh(
        option,
        default = "String::from(\"wikidata\")",
        description = "identity scheme"
    )]
    scheme: String,
    #[argh(option, description = "why — the only place this is recorded")]
    note: Option<String>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "unveto")]
#[argh(description = "lift a veto; the next resolve may claim the identity again")]
struct UnvetoArgs {
    #[argh(positional, description = "verdict id (verdicts list)")]
    id: i32,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "reproject")]
#[argh(description = "recompute projections for live entities holding no live claim")]
struct ReprojectArgs {
    #[argh(switch)]
    #[argh(description = "reproject every live entity, not only the ones holding no live claim")]
    all: bool,
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
    AutoMerge(ProximityAutoMergeArgs),
    List(ProximityListArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "scan")]
#[argh(description = "record every pair that shares an identity; changes nothing else")]
struct ProximityScanArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "auto-merge")]
#[argh(description = "fold the open proximities that look like one thing, on their own")]
struct ProximityAutoMergeArgs {}

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
    #[argh(
        option,
        description = "only this cause: repair, proximity_auto, replayed"
    )]
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
    #[argh(switch)]
    #[argh(
        description = "consider the newest entities first, so a short run serves fresh captures"
    )]
    newest: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "stats")]
#[argh(description = "counts: observations, linked share, entities by status, duplicate rate")]
struct StatsArgs {}

/// Print one entity's walk through the pipeline.
fn print_inspection(report: &knowledge::eval::inspect::Report) {
    let entity = &report.entity;
    println!(
        "entity {} · {} [{}] · {}",
        entity.id, entity.name, entity.entity_type, entity.status
    );
    if let Some(description) = &entity.description {
        println!("  description: {description}");
    }
    if let Some(hint) = &entity.search_query {
        println!("  hint: {hint}");
    }
    println!(
        "  surfaces it answers to: {:?}",
        entity.surfaces().collect::<Vec<_>>()
    );

    println!("\n-- recall (knowledge_blocking_index) --");
    if report.candidates.is_empty() {
        println!("  no candidates");
    }
    for candidate in &report.candidates {
        let mut via = Vec::new();
        if candidate.by_surface {
            via.push("surface");
        }
        if candidate.by_words {
            via.push("words");
        }
        let via = if via.is_empty() {
            "-".to_string()
        } else {
            via.join("+")
        };
        println!(
            "  {:>7}  {:<38} [{}] via {via}",
            candidate.id, candidate.name, candidate.entity_type
        );
    }

    println!("\n-- rank (what the linker would attach to) --");
    if report.ranked.is_empty() {
        println!("  nothing qualifies → Create");
    }
    for (id, name, score) in &report.ranked {
        println!("  {id:>7}  {name:<38} {score:.3}");
    }

    println!("\n-- resolve ladder --");
    for step in &report.steps {
        let cache = match step.cache {
            dreamscroll::fetch::cache::State::Fresh => "cached",
            dreamscroll::fetch::cache::State::Expired => "expired → miss",
            dreamscroll::fetch::cache::State::Missing => "not asked",
        };
        let outcome = step.outcome.map(|outcome| outcome.as_str()).unwrap_or("—");
        println!("  {:<16} {:?}  [{cache} {outcome}]", step.kind, step.query);
        for (id, label, description) in &step.top {
            println!("      {id:<12} {label:<34} {description}");
        }
        match &step.decision {
            Some((qid, method)) if step.vetoed => {
                println!("      ⊘ vetoed {qid} via {method} (standing verdict)")
            }
            Some((qid, method)) => println!("      → claimed {qid} via {method}"),
            None => println!("      ✗ {}", step.refusal),
        }
    }

    println!("\n-- held now --");
    match &report.claim {
        Some(claim) => println!(
            "  {} {} · asserted by {} · method {}",
            claim.identity,
            claim.label.as_deref().unwrap_or("(no label)"),
            claim.asserted_by,
            claim.method.as_deref().unwrap_or("-")
        ),
        None => println!("  no claim"),
    }
}

/// Ask the authorities what a thing is, cache first.
async fn resolve_identity(db: &database::DbHandle, args: ResolveArgs) -> anyhow::Result<()> {
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
        (false, None) => knowledge::identity::DEFAULT_MAX_REQUESTS,
    };
    let options = knowledge::identity::Options {
        limit: args.limit,
        max_requests,
        offline: args.offline,
        only: args.entity,
        min_interval: std::time::Duration::from_millis(args.min_interval_ms),
        newest_first: args.newest,
    };
    let engine = knowledge::identity::ResolutionEngine::new(db, options)
        .with(Box::new(knowledge::identity::wikidata::Wikidata::new()?));
    let report = engine.run().await?;

    if report.offline {
        println!("offline — cache only, no transport calls");
    } else {
        println!("budget {max_requests} requests");
    }
    println!(
        "candidates {} · requested {} · cache hits {}",
        report.candidates, report.stats.requested, report.stats.cache_hits
    );
    println!(
        "fetched {} · failed {} · claims {} · entities resolved {} · vetoed {}",
        report.stats.fetched,
        report.stats.failed,
        report.claims,
        report.entities_resolved,
        report.vetoed
    );
    match report.refusal {
        Some(dreamscroll::fetch::Refusal::Budget) => {
            println!("budget exhausted at {max_requests} requests");
        }
        // WDQS 429: honour Retry-After rather than risk a temporary ban.
        Some(dreamscroll::fetch::Refusal::RateLimited { retry_after_secs }) => {
            match retry_after_secs {
                Some(seconds) => println!(
                    "rate limited by the provider — stopped; retry in {seconds}s \
                     (or lower --min-interval-ms / --max-requests)"
                ),
                None => println!(
                    "rate limited by the provider — stopped; it gave no retry time \
                     (or lower --min-interval-ms / --max-requests)"
                ),
            }
        }
        None => {}
    }
    Ok(())
}

/// `eval blocker`: the blocking replay. Recall is measured leave-future-out — a candidate only
/// counts if its entity already had an earlier member — so this answers "was the right thing on
/// the table when it mattered", not "can the index still find it now".
async fn eval_blocker(db: &database::DbHandle) -> anyhow::Result<()> {
    let report = knowledge::eval::replay_blocker(db).await?;
    println!(
        "observations {} (superseded {} skipped) · first member {} · eligible {}",
        report.observations, report.superseded, report.new_entity, report.eligible
    );
    println!(
        "index coverage {}/{} ({:.1}%) (expected 100%) · starved {} · ambiguous {}",
        report.hit,
        report.eligible,
        report.index_coverage(),
        report.starved,
        report.ambiguous
    );
    // A post-hoc merge is not a blocking failure — the pair did not exist when the observation
    // arrived — so it is listed but kept out of the denominator.
    println!(
        "earlier-members recall {}/{} ({:.1}%) · {} misses: {} post-hoc merges (excluded), {} unexplained",
        report.prior_hit,
        report.attributable(),
        report.attributable_recall(),
        report.misses(),
        report.merged,
        report.unexplained_misses()
    );
    for miss in report.prior_misses.iter().take(25) {
        println!(
            "  blocked  {:<32} -> #{} {}  [{}]",
            miss.name,
            miss.expected,
            miss.expected_name,
            if miss.merged {
                "post-hoc merge"
            } else {
                "unexplained"
            }
        );
    }
    let sizes: Vec<String> = report
        .sizes
        .iter()
        .map(|(size, count)| format!("{size}:{count}"))
        .collect();
    println!("candidates per eligible observation: {}", sizes.join(" · "));
    for miss in report.misses.iter().take(20) {
        println!(
            "  miss  {:<30} -> #{} {} (offered {})",
            miss.name, miss.expected, miss.expected_name, miss.offered
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
            let report = knowledge::link_all_pending(&db).await?;
            println!(
                "considered {} · already linked {} · linked {} · created {} · left pending {} · vetoed {}",
                report.considered,
                report.already_linked,
                report.attached,
                report.created,
                report.left_pending,
                report.vetoed
            );
        }
        Command::Merge(args) => {
            let store = knowledge::KnowledgeGraph::new(&db);
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
            let cause = knowledge::verdicts::Cause::Repair;
            match store.merge(args.loser, args.winner, cause).await? {
                knowledge::graph::merge::Outcome::Merged => {
                    println!("merged; {} is now a tombstone", args.loser)
                }
                knowledge::graph::merge::Outcome::AlreadyMerged => {
                    println!("already merged; nothing to do")
                }
                knowledge::graph::merge::Outcome::RefusedSharedIllumination => {
                    println!("refused at merge time — the entities changed under us")
                }
            }
        }
        Command::Verdicts(args) => match args.command {
            VerdictsCommand::Replay(_) => {
                let store = knowledge::KnowledgeGraph::new(&db);
                let report = store.replay_verdicts().await?;
                println!(
                    "applied {} · already {} · gone {} · refused {} · withdrawn {} · settled {}",
                    report.applied,
                    report.already,
                    report.gone,
                    report.refused,
                    report.withdrawn,
                    report.settled
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
                    let decided = match row.kind.as_str() {
                        "veto" => format!("{}  \u{2298}  {}", row.subject, row.identity),
                        _ => format!("{}  \u{21c4}  {}", row.winner, row.loser),
                    };
                    println!(
                        "{:>4}  gen {:<3} {:<15} {:<5} {}{}",
                        row.id,
                        row.generation,
                        row.cause,
                        row.kind,
                        decided,
                        if row.note.is_empty() {
                            String::new()
                        } else {
                            format!("   — {}", row.note)
                        }
                    );
                }
            }
        },
        Command::Proximity(args) => {
            let store = knowledge::KnowledgeGraph::new(&db);
            let scoring = knowledge::linker::heuristic::Heuristic::scoring();
            match args.command {
                ProximityCommand::Scan(_) => {
                    println!(
                        "raised {} proximities",
                        knowledge::proximity::scan(&db.conn, &scoring).await?
                    );
                }
                ProximityCommand::AutoMerge(_) => {
                    let report =
                        knowledge::proximity::auto_merge(&db.conn, &store, &scoring).await?;
                    println!(
                        "auto-merged {} · refused {} · moot {} · undecided {}",
                        report.merged, report.refused, report.moot, report.undecided
                    );
                }
                ProximityCommand::List(args) => {
                    let rows = stats::proximities::proximity_list(&db, args.limit).await?;
                    for row in rows {
                        let score = row.score.map_or("-".to_string(), |s| format!("{s:.2}"));
                        let decided = if row.decided { "same" } else { "" };
                        println!(
                            "{:>5}  {decided:<5}{score:>5}  {:<16}  {:<15}  #{:<6} {}  \
                             \u{21c4}  #{:<6} {}  [{}]",
                            row.id,
                            row.note,
                            row.basis,
                            row.holder_id,
                            row.holder,
                            row.counterpart_id,
                            row.counterpart,
                            row.identity
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
                "unlinked {} observations · deleted {} claims, {} proximities · \
                 deleted {} entities",
                report.unlinked, report.claims, report.proximities, report.entities
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
        Command::Index(args) => match args.command {
            IndexCommand::Rebuild(_) => {
                let rows = knowledge::blocker::index::rebuild(&db).await?;
                println!("rebuilt {rows} index rows");
            }
            IndexCommand::Check(_) => {
                let report = knowledge::blocker::index::check(&db).await?;
                println!(
                    "indexed {} · stale {} · missing {} · orphaned {} · {}",
                    report.indexed,
                    report.stale,
                    report.missing,
                    report.orphaned,
                    if report.is_current() {
                        "current"
                    } else {
                        "REBUILD NEEDED"
                    }
                );
            }
        },
        Command::Eval(args) => match args.command {
            EvalCommand::Blocker(_) => eval_blocker(&db).await?,
            EvalCommand::Inspect(args) => {
                let report = knowledge::eval::inspect(&db, &args.target).await?;
                match report {
                    Some(report) => print_inspection(&report),
                    None => println!("nothing matches {:?}", args.target),
                }
            }
        },
        Command::Identity(args) => match args.command {
            IdentityCommand::Resolve(args) => resolve_identity(&db, args).await?,
            IdentityCommand::Reproject(args) => {
                let scope = if args.all {
                    knowledge::graph::ReprojectScope::All
                } else {
                    knowledge::graph::ReprojectScope::Stale
                };
                let total = knowledge::KnowledgeGraph::new(&db).reproject(scope).await?;
                println!(
                    "reprojected {total} {}",
                    if args.all {
                        "live entities"
                    } else {
                        "live entities holding no live claim"
                    }
                );
            }
            IdentityCommand::Veto(args) => {
                let store = knowledge::KnowledgeGraph::new(&db);
                let report = store
                    .veto(
                        args.entity,
                        &args.scheme,
                        &args.identity,
                        args.note.as_deref(),
                    )
                    .await?;
                let effect = if report.withdrawn == 0 {
                    "settled".to_string()
                } else {
                    format!("withdrawn {}", report.withdrawn)
                };
                println!(
                    "{} · {effect}",
                    if report.recorded {
                        "veto recorded"
                    } else {
                        "veto already recorded"
                    }
                );
            }
            IdentityCommand::Unveto(args) => {
                let store = knowledge::KnowledgeGraph::new(&db);
                let lifted = store.unveto(args.id).await?;
                println!(
                    "{} — the next resolve may claim the identity again",
                    if lifted {
                        "veto lifted"
                    } else {
                        "no such veto"
                    }
                );
            }
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
