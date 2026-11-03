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
                "considered {} · already linked {} · linked {} ({} by claim) · created {} · discarded {} · vetoed {}",
                report.considered,
                report.already_linked,
                report.attached,
                report.attached_by_claim,
                report.created,
                report.discarded_on_conflict,
                report.vetoed
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
        Command::Reset(args) => {
            if !args.yes {
                anyhow::bail!("reset empties the projected graph; pass --yes to confirm");
            }
            let report = knowledge::snapshot::reset(&db).await?;
            match &report.snapshot {
                Some(snapshot) => println!(
                    "snapshot {} · entities {} · identities {} · claims {} · merges {} · collisions {} · members {}",
                    snapshot.suffix,
                    report.snapshot_entities,
                    report.snapshot_identities,
                    report.snapshot_claims,
                    report.snapshot_merges,
                    report.snapshot_collisions,
                    report.snapshot_members
                ),
                None => println!("nothing to snapshot"),
            }
            println!(
                "unlinked {} observations · deleted {} claims, {} merges, {} collisions · \
                 deleted {} entities",
                report.unlinked, report.claims, report.merges, report.collisions, report.entities
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
        Command::Resolve(args) => {
            // A budget of zero makes no transport calls, which reads like a successful run that
            // resolved nothing. That is a mode, not a number.
            if args.offline && args.max_requests.is_some() {
                anyhow::bail!("--offline and --max-requests are mutually exclusive");
            }
            if args.max_requests == Some(0) {
                anyhow::bail!(
                    "--max-requests 0 makes no transport calls; pass --offline to ask for that"
                );
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
            let engine = knowledge::resolve::ResolutionEngine::new(&db, options)
                .with(Box::new(knowledge::resolve::wikidata::Wikidata::new()?));
            let report = engine.run().await?;

            if report.offline {
                println!("offline — cache only, no transport calls");
            } else {
                println!("budget {} requests", max_requests);
            }
            println!(
                "candidates {} · planned {} · cache hits {}",
                report.candidates, report.planned, report.cache_hits
            );
            println!(
                "fetched {} · failed {} · claims {} · entities resolved {} · merges {} · collisions {}",
                report.fetched,
                report.failed,
                report.claims,
                report.entities_resolved,
                report.merges,
                report.collisions
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
        }
        Command::Stats(_) => {
            let stats = stats::knowledge_stats(&db).await?;
            println!("observations        {:>7}", stats.observations);
            println!("  linked            {:>7}", stats.linked);
            println!("knowledge entities  {:>7}", stats.entities);
            println!("  provisional       {:>7}", stats.provisional);
            println!("  identified        {:>7}", stats.identified);
            println!("  contested         {:>7}", stats.contested);
            println!("duplicate rate      {:>7.3}", stats.duplicate_rate());
            println!("resolution");
            println!("  with authority claim {:>5}", stats.with_claim);
            println!(
                "  platform presence      {} projected · {} from members",
                stats.with_platform, stats.platform_from_members
            );
            println!(
                "  cache {} with candidates · {} no match · {} failed · {} expired",
                stats.cache_found, stats.cache_empty, stats.cache_failed, stats.cache_expired
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
    attached_by_claim: usize,
    created: usize,
    discarded_on_conflict: usize,
    vetoed: u32,
}

impl LinkAllPendingReport {
    fn update(&mut self, part: knowledge::LinkStats) {
        self.attached += part.attached;
        self.attached_by_claim += part.attached_by_claim;
        self.created += part.created;
        self.discarded_on_conflict += part.discarded_on_conflict;
        self.vetoed += part.vetoed;
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
