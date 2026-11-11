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
    Entity(EntityArgs),
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
    #[argh(switch, description = "print JSON")]
    json: bool,
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
    #[argh(switch, description = "print JSON")]
    json: bool,
    #[argh(option, description = "why these are one thing; stored on the verdict")]
    note: Option<String>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "entity")]
#[argh(description = "one knowledge entity and its evidence")]
struct EntityArgs {
    #[argh(subcommand)]
    command: EntityCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum EntityCommand {
    Show(EntityShowArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "show")]
#[argh(description = "members, claims and projected columns for one entity")]
struct EntityShowArgs {
    #[argh(positional, description = "knowledge entity id")]
    id: i32,
    #[argh(switch, description = "print JSON")]
    json: bool,
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
    Distinct(DistinctArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "scan")]
#[argh(description = "record every pair that shares an identity; changes nothing else")]
struct ProximityScanArgs {}

#[derive(FromArgs)]
#[argh(subcommand, name = "auto-merge")]
#[argh(description = "fold the open proximities that look like one thing, on their own")]
struct ProximityAutoMergeArgs {
    #[argh(switch, short = 'y')]
    #[argh(description = "apply the folds (otherwise it is a dry run)")]
    yes: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "distinct")]
#[argh(description = "record that two entities are deliberately separate")]
struct DistinctArgs {
    #[argh(positional, description = "first knowledge entity id")]
    a: i32,
    #[argh(positional, description = "second knowledge entity id")]
    b: i32,
    #[argh(option, description = "why; stored on the verdict")]
    note: Option<String>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "list")]
#[argh(description = "show what is in the table, most likely first")]
struct ProximityListArgs {
    #[argh(option, default = "50", description = "maximum rows to show")]
    limit: usize,
    #[argh(
        option,
        description = "only this status: open, merge_pending, merged, kept_apart"
    )]
    status: Option<String>,
    #[argh(switch, description = "only open pairs")]
    open: bool,
    #[argh(switch, description = "print JSON")]
    json: bool,
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
    #[argh(switch, description = "print JSON")]
    json: bool,
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
    description = "snapshot the entities, evidence and membership, then empty them; previews without --yes"
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
    #[argh(switch)]
    #[argh(
        description = "with --entity, retract this provider's old claims that the new answer did \
                       not re-propose; requires --entity"
    )]
    replace: bool,
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
    if let Some(origin) = entity.origin_observation_id {
        println!("  origin observation: {origin}");
    }
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
            "  {} {} · provided by {} · method {}",
            claim.identity,
            claim.label.as_deref().unwrap_or("(no label)"),
            claim.provider,
            claim.method.as_deref().unwrap_or("-")
        ),
        None => println!("  no claim"),
    }
}

/// Print one entity's evidence: members, claims, projected columns.
fn print_entity(detail: &stats::entity::Detail) {
    let entity = &detail.entity;
    println!(
        "entity {} · {} [{}] · {}",
        entity.id,
        entity.name,
        entity.entity_type,
        entity.status.as_str()
    );
    if let Some(origin) = entity.origin_observation_id {
        println!("  origin observation: {origin}");
    }
    if let Some(merged) = entity.merged_into_id {
        println!("  merged into: {merged}");
    }
    if let Some(description) = &entity.description {
        println!("  description: {description}");
    }
    if let Some(hint) = &entity.search_query {
        println!("  hint: {hint}");
    }
    println!("  surfaces: {:?}", entity.surfaces().collect::<Vec<_>>());
    println!("  confidence: {:?}", entity.confidence);

    println!("\n-- members ({}) --", detail.members.len());
    for member in &detail.members {
        println!(
            "  obs {} · capture {} · illumination {} · {} [{}] · handle {:?}",
            member.id,
            member.capture_id,
            member.illumination_id,
            member.name,
            member.entity_type,
            member.platform_handle
        );
    }

    println!("\n-- claims ({}) --", detail.claims.len());
    for claim in &detail.claims {
        println!(
            "  {}:{}  {}  basis {} · provider {} · confidence {:?} · {}{}",
            claim.scheme,
            claim.external_id,
            claim.label.as_deref().unwrap_or("(no label)"),
            claim.basis,
            claim.provider,
            claim.confidence,
            claim.method.as_deref().unwrap_or("-"),
            if claim.retracted_at.is_some() {
                " · retracted"
            } else {
                ""
            }
        );
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
    if args.replace && args.entity.is_none() {
        anyhow::bail!("--replace requires --entity; a corpus-wide replace is a rebuild");
    }
    let max_requests = match (args.offline, args.max_requests) {
        (true, _) => 0,
        (false, Some(n)) => n,
        (false, None) => knowledge::identity::DEFAULT_MAX_REQUESTS,
    };
    let options = knowledge::identity::Options {
        include_already_claimed: args.entity.is_some(),
        replace: args.replace,
    };
    let engine = knowledge::identity::ResolutionEngine::new(
        knowledge::KnowledgeGraph::new(db),
        options,
        vec![Box::new(knowledge::identity::wikidata::Wikidata::new()?)],
    )?;
    let cache = dreamscroll::fetch::FetchCache::new(&db.conn);
    let allowance = (!args.offline).then(|| {
        dreamscroll::fetch::Allowance::new(
            max_requests,
            std::time::Duration::from_millis(args.min_interval_ms),
        )
    });
    let report = match args.entity {
        Some(id) => {
            let Some(before) = stats::entity::detail(db, id).await? else {
                anyhow::bail!("no knowledge entity {id}");
            };
            anyhow::ensure!(
                before.entity.merged_into_id.is_none(),
                "knowledge entity {id} is a tombstone; resolve the live entity"
            );
            let report = engine
                .resolve(
                    std::slice::from_ref(&before.entity),
                    &cache,
                    allowance.as_ref(),
                )
                .await?;
            let after = stats::entity::detail(db, id)
                .await?
                .expect("the entity exists");
            println!("entity {id} claims");
            let live: Vec<_> = after
                .claims
                .iter()
                .filter(|claim| claim.retracted_at.is_none())
                .collect();
            if live.is_empty() {
                println!("  (none)");
            }
            for claim in live {
                let held = before.claims.iter().any(|old| {
                    old.scheme == claim.scheme
                        && old.external_id == claim.external_id
                        && old.retracted_at.is_none()
                });
                println!(
                    "  {} {}:{}  {}  provider {}",
                    if held { "*" } else { "→" },
                    claim.scheme,
                    claim.external_id,
                    claim.method.as_deref().unwrap_or("-"),
                    claim.provider
                );
            }
            report
        }
        None => {
            knowledge::resolve_all_pending(
                db,
                &engine,
                &cache,
                allowance.as_ref(),
                args.limit,
                args.newest,
            )
            .await?
        }
    };

    if report.offline {
        println!("offline — cache only, no transport calls");
    } else {
        println!("budget {max_requests} requests");
    }
    println!(
        "considered {} · requested {} · cache hits {}",
        report.considered, report.traffic.requested, report.traffic.cache_hits
    );
    println!(
        "fetched {} · failed {} · claims {} · entities resolved {} · vetoed {}",
        report.traffic.fetched,
        report.traffic.failed,
        report.written,
        report.entities_resolved,
        report.vetoed
    );
    for provider in &report.by_provider {
        println!(
            "  {:<16} batch {:>5} · requested {:>5} · cache {:>5} · fetched {:>4} · claims {:>4} · vetoed {} · replaced {}",
            provider.provider,
            provider.batch,
            provider.traffic.requested,
            provider.traffic.cache_hits,
            provider.traffic.fetched,
            provider.written,
            provider.vetoed,
            provider.replaced
        );
    }
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

fn exit_with(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(code);
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
            let mut report = serde_json::json!({
                "action": "merge",
                "loser": args.loser,
                "winner": args.winner,
                "observations": preview.observations,
                "evidence": preview.evidence,
                "shared_illuminations": preview.shared_illuminations,
                "outcome": "dry_run",
                "note": args.note,
            });
            if !args.json {
                println!(
                    "merge {} -> {} · observations {} · evidence {} · shared illuminations {}",
                    args.loser,
                    args.winner,
                    preview.observations,
                    preview.evidence,
                    preview.shared_illuminations
                );
            }
            if preview.shared_illuminations > 0 {
                report["outcome"] = serde_json::json!("refused");
                report["reason"] = serde_json::json!("shared_illumination");
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!(
                        "refused: both entities appear in the same illumination, and one extraction \
                         emitting both means two referents"
                    );
                }
                exit_with(2);
            }
            if !args.yes {
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!("dry run; pass --yes to apply");
                }
                return Ok(());
            }
            let cause = knowledge::verdicts::Cause::Repair;
            match store
                .merge_with_note(args.loser, args.winner, cause, args.note.as_deref())
                .await?
            {
                knowledge::graph::merge::Outcome::Merged => {
                    report["outcome"] = serde_json::json!("merged");
                    if args.json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        println!("merged; {} is now a tombstone", args.loser);
                    }
                }
                knowledge::graph::merge::Outcome::AlreadyMerged => {
                    report["outcome"] = serde_json::json!("already_merged");
                    if args.json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        println!("already merged; nothing to do");
                    }
                }
                knowledge::graph::merge::Outcome::RefusedSharedIllumination => {
                    report["outcome"] = serde_json::json!("refused");
                    report["reason"] = serde_json::json!("state_changed");
                    if args.json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        println!("refused at merge time — the entities changed under us");
                    }
                    exit_with(3);
                }
            }
        }
        Command::Entity(args) => match args.command {
            EntityCommand::Show(args) => {
                let Some(detail) = stats::entity::detail(&db, args.id).await? else {
                    anyhow::bail!("no knowledge entity {}", args.id);
                };
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&detail.to_json())?);
                } else {
                    print_entity(&detail);
                }
            }
        },
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
                if args.json {
                    let values: Vec<serde_json::Value> =
                        rows.iter().map(|row| row.to_json()).collect();
                    println!("{}", serde_json::to_string_pretty(&values)?);
                } else {
                    for row in &rows {
                        let decided = match row.kind.as_str() {
                            "veto_identity" => {
                                format!("{}  \u{2298}  {}", row.subject, row.identity)
                            }
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
                ProximityCommand::AutoMerge(args) => {
                    let report =
                        knowledge::proximity::auto_merge(&db.conn, &store, &scoring, args.yes)
                            .await?;
                    if args.yes {
                        println!(
                            "auto-merged {} · refused {} · moot {} · kept apart {} · undecided {}",
                            report.merged,
                            report.refused,
                            report.moot,
                            report.kept_apart,
                            report.undecided
                        );
                    } else {
                        println!(
                            "would merge {} · refused {} · moot {} · kept apart {} · undecided {} · dry run; pass --yes",
                            report.merged,
                            report.refused,
                            report.moot,
                            report.kept_apart,
                            report.undecided
                        );
                    }
                }
                ProximityCommand::Distinct(args) => {
                    let recorded = store.distinct(args.a, args.b, args.note.as_deref()).await?;
                    println!(
                        "{} · {} \u{21c4} {}",
                        if recorded {
                            "distinct recorded"
                        } else {
                            "distinct already recorded"
                        },
                        args.a,
                        args.b
                    );
                }
                ProximityCommand::List(args) => {
                    let filter = match (args.open, args.status.as_deref()) {
                        (true, Some(_)) => {
                            anyhow::bail!("--open and --status are mutually exclusive")
                        }
                        (true, None) => Some(stats::proximities::Status::Open),
                        (false, Some(value)) => Some(
                            stats::proximities::Status::parse(value).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "unknown status {value:?}; expected open, merge_pending, merged or kept_apart"
                                )
                            })?,
                        ),
                        (false, None) => None,
                    };
                    let rows = stats::proximities::proximity_list(&db, args.limit, filter).await?;
                    if args.json {
                        let values: Vec<serde_json::Value> =
                            rows.iter().map(|row| row.to_json()).collect();
                        println!("{}", serde_json::to_string_pretty(&values)?);
                    } else {
                        println!(
                            "{:>5}  {:<13}  {:<14}  {:>5}  {:<16}  {:<15}  pair",
                            "row", "status", "why", "score", "type", "basis"
                        );
                        for row in rows {
                            let score = row.score.map_or("-".to_string(), |s| format!("{s:.2}"));
                            let status = row.status.as_str();
                            let why = match (row.status, row.why) {
                                (stats::proximities::Status::Merged, Some(why)) => why.as_str(),
                                (stats::proximities::Status::Merged, None) => "?",
                                (_, Some(why)) => why.as_str(),
                                (_, None) => "",
                            };
                            println!(
                                "{:>5}  {:<13}  {:<14}  {score:>5}  {:<16}  {:<15}  #{:<6} {}  \
                                 \u{21c4}  #{:<6} {}  [{}]",
                                row.id,
                                status,
                                why,
                                row.type_note,
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
        }
        Command::Reset(args) => {
            if !args.yes {
                let preview = knowledge::snapshot::preview(&db).await?;
                println!(
                    "would snapshot entities {} · identities {} · claims {} · proximities {} · \
                     members {} · pass --yes to reset",
                    preview.entities,
                    preview.identities,
                    preview.claims,
                    preview.proximities,
                    preview.members
                );
                return Ok(());
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
            if report.pruned_snapshots > 0 {
                println!("pruned {} old snapshot tables", report.pruned_snapshots);
            }
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
                    Some(report) => {
                        if args.json {
                            println!("{}", serde_json::to_string_pretty(&report.to_json())?);
                        } else {
                            print_inspection(&report);
                        }
                    }
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
                println!(
                    "{} · entity {} · {}:{} · withdrew {} · note {:?}",
                    if report.recorded {
                        "veto_identity recorded"
                    } else {
                        "veto_identity already recorded"
                    },
                    args.entity,
                    args.scheme,
                    args.identity,
                    report.withdrawn,
                    args.note.as_deref().unwrap_or(""),
                );
            }
            IdentityCommand::Unveto(args) => {
                let store = knowledge::KnowledgeGraph::new(&db);
                let lifted = store.unveto(args.id).await?;
                println!(
                    "{} · verdict {}",
                    if lifted {
                        "veto_identity lifted"
                    } else {
                        "no such veto_identity"
                    },
                    args.id
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
            println!("platform links");
            println!(
                "  {:<14} {:>12} {:>10} {:>8} {:>8} {:>9}",
                "platform", "observations", "identities", "entities", "display", "captures"
            );
            for group in &stats.platforms {
                println!(
                    "  {:<14} {:>12} {:>10} {:>8} {:>8} {:>9}",
                    group.platform,
                    group.observations,
                    group.identities,
                    group.entities,
                    group.display_names,
                    group.captures
                );
            }
            let linked: u64 = stats.platforms.iter().map(|group| group.observations).sum();
            println!("  total {linked} observations with a platform link");
            println!("claims per entity");
            println!(
                "  entities {} · 0 claims {} · 1 claim {} · 2 claims {} · 3+ claims {}",
                stats.claim_counts.entities,
                stats.claim_counts.zero,
                stats.claim_counts.one,
                stats.claim_counts.two,
                stats.claim_counts.three_plus
            );
            let number = |value: Option<f64>| {
                value
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "-".to_string())
            };
            println!("confidence");
            println!(
                "  min {} · avg {} · max {} · unknown {}",
                number(stats.confidence.min),
                number(stats.confidence.average),
                number(stats.confidence.max),
                stats.confidence.unknown
            );
            println!("resolution");
            println!("  with authority claim {:>5}", stats.with_claim);
            println!("  claims by provider / scheme");
            println!(
                "    {:<16} {:<18} {:>10} {:>7} {:>9} {:>10}",
                "provider", "scheme", "identities", "claims", "entities", "retracted"
            );
            for group in &stats.claims_by {
                println!(
                    "    {:<16} {:<18} {:>10} {:>7} {:>9} {:>10}",
                    group.provider,
                    group.scheme,
                    group.identities,
                    group.live_claims,
                    group.entities,
                    group.retracted
                );
            }
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
