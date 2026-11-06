//! Replay the resolver's whole decision over the answer cache, for the current corpus.
//!
//! Run: `cargo run --example eval_corpus [-- --verbose]`
//!
//! No transport: every request is answered from `resolution_cache`, so a change to `ranking.rs`
//! or the admission rules can be measured in seconds instead of 640 fetches. It walks the same
//! ladder the driver walks — the first request that yields a claim wins — and reports what each
//! undecided entity would need.
//!
//! The point is iteration: change one gate, re-run, diff the claims.

use dreamscroll::knowledge::resolve::cache::Outcome;
use dreamscroll::knowledge::resolve::wikidata::Wikidata;
use dreamscroll::knowledge::resolve::Raw;
use dreamscroll::model;
use dreamscroll::{config, database};
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

/// Entities the driver would consider: live, and with no ontology claim (a platform handle alone
/// still deserves a QID).
const CANDIDATES: &str = "
SELECT e.id FROM knowledge_entities e
 WHERE e.merged_into_id IS NULL
   AND NOT EXISTS (
       SELECT 1 FROM knowledge_claims c
         JOIN knowledge_identities i ON i.id = c.knowledge_identity_id
        WHERE c.knowledge_entity_id = e.id AND c.retracted_at IS NULL
          AND i.source <> 'platform')
 ORDER BY e.id";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    let verbose = std::env::args().any(|arg| arg == "--verbose");
    let conn = database::connect_postgres_db(&config::make_postgres()?).await?;
    let engine = Wikidata::new()?;

    let ids: Vec<i32> = conn
        .query_all_raw(Statement::from_string(DatabaseBackend::Postgres, CANDIDATES))
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<i32>(0))
        .collect::<Result<_, _>>()?;

    let (mut claimed, mut undecided, mut uncached) = (0usize, 0usize, 0usize);
    let mut by_method: std::collections::BTreeMap<String, usize> = Default::default();
    let mut gained: Vec<(String, String, String)> = Vec::new();

    for id in ids {
        let Some(view) = model::knowledge_entity::Entity::find_by_id(id)
            .one(&conn)
            .await?
        else {
            continue;
        };

        let mut decided = None;
        let mut missed_request = false;
        // The ladder the driver walks: the first pass, then the search fallback for whatever it
        // left undecided. A harness that skips the fallback measures nothing — every claim the
        // corpus actually earned came from it.
        let mut ladder = engine.plan(&view);
        ladder.extend(engine.plan_fallback(&view));
        for request in ladder {
            let cached = conn
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT outcome, payload::text FROM resolution_cache \
                      WHERE provider = $1 AND kind = $2 AND shape = $3 AND query = $4 \
                        AND expires_at > now() LIMIT 1",
                    [
                        request.provider.clone().into(),
                        request.kind.clone().into(),
                        request.shape.clone().into(),
                        request.query.clone().into(),
                    ],
                ))
                .await?;
            let Some(row) = cached else {
                missed_request = true;
                continue;
            };
            let outcome: String = row.try_get_by_index(0)?;
            let payload: Option<String> = row.try_get_by_index(1)?;
            let raw = Raw {
                outcome: if outcome == "found" {
                    Outcome::Found
                } else {
                    Outcome::Empty
                },
                payload: payload
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?,
            };
            let parsed = engine.parse(&view, &request, &raw);
            if let Some(claim) = parsed.into_iter().next() {
                decided = Some((claim.id, claim.method));
                break;
            }
        }

        match decided {
            Some((qid, method)) => {
                claimed += 1;
                *by_method.entry(method.clone()).or_default() += 1;
                if verbose {
                    gained.push((view.name.clone(), qid, method));
                }
            }
            None if missed_request => uncached += 1,
            None => undecided += 1,
        }
    }

    if verbose {
        for (name, qid, method) in &gained {
            println!("{qid:<12} {method:<14} {name}");
        }
        println!();
    }
    println!("candidates {} · claimed {claimed} · undecided {undecided} · not cached {uncached}", claimed + undecided + uncached);
    println!("by method: {by_method:?}");
    Ok(())
}
