//! Compare the SQL blocker against the in-memory one, over the whole corpus.
//!
//! `NameBlocker` is the oracle: it holds every entity and answers from a map, so for a corpus this
//! size it is the ground truth for "which entities share a key with this observation". `SqlBlocker`
//! reads `knowledge_index`. Anything the SQL side misses is a recall bug; anything it adds is noise
//! the oracle would not have handed to `Rank`.
//!
//! The two generations are measured apart as well as together, because the second one (`word_keys`)
//! is a weaker claim about identity and exists only for parity with the in-memory blocker. See the
//! backlog: it has to earn its place, not inherit it.

use std::collections::BTreeSet;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, QueryOrder, Statement};

use crate::database::DbHandle;
use crate::knowledge::blocker::Blocker;
use crate::knowledge::heuristic::NameBlocker;
use crate::knowledge::normalize;
use crate::knowledge::sql_blocker::SqlBlocker;
use crate::model;

/// What one comparison found.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub observations: usize,
    /// Candidate pairs the oracle offered.
    pub oracle_pairs: usize,
    /// Candidate pairs the SQL blocker offered.
    pub sql_pairs: usize,
    /// Offered by the oracle, missed by SQL. Must be zero: a miss here is a recall bug.
    pub missing: usize,
    /// Offered by SQL, absent from the oracle. Noise `Rank` would have to refuse.
    pub extra: usize,
    /// Observations with a *foreign* surface hit — some other entity wears the same folded name.
    pub surface_hits: usize,
    /// Observations with no foreign surface hit but at least one foreign word hit: `Dreams Train`
    /// for `Train Dreams`. This is the number the backlog item is about, and the own entity is
    /// excluded because a linked observation always matches it, which would make every observation
    /// look like a surface hit.
    pub word_only: usize,
    /// The first few word-only observations, so the number can be read rather than trusted.
    pub examples: Vec<String>,

    /// Wall time to build the in-memory blocker **once**. That is what a new process pays at
    /// startup, and what today's link task pays *per illumination* — the cost that scales with the
    /// corpus, not with the work.
    pub oracle_build_ms: f64,
    /// Mean milliseconds per `candidates` call.
    pub oracle_query_ms: f64,
    pub sql_query_ms: f64,
    /// Mean observations in an illumination, so the two can be compared the way the task runs.
    pub observations_per_illumination: f64,
}

/// Replay every observation through both blockers and compare.
pub async fn replay(db: &DbHandle, limit: Option<usize>) -> anyhow::Result<Report> {
    let building = std::time::Instant::now();
    let oracle = NameBlocker::new(db).await?;
    let build = building.elapsed();

    let sql = SqlBlocker::new(db);

    let observations = model::user_observed_entity::Entity::find()
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(&db.conn)
        .await?;

    let mut report = Report {
        oracle_build_ms: build.as_secs_f64() * 1e3,
        ..Default::default()
    };
    let (mut oracle_nanos, mut sql_nanos) = (0u128, 0u128);
    let mut illuminations = std::collections::BTreeSet::new();
    for observation in observations.iter().take(limit.unwrap_or(usize::MAX)) {
        illuminations.insert(observation.illumination_id);
        report.observations += 1;

        let started = std::time::Instant::now();
        let expected: BTreeSet<i32> = oracle
            .candidates(observation)
            .await
            .into_iter()
            .map(|row| row.id)
            .collect();
        oracle_nanos += started.elapsed().as_nanos();

        let started = std::time::Instant::now();
        let got: BTreeSet<i32> = sql
            .candidates(observation)
            .await
            .into_iter()
            .map(|row| row.id)
            .collect();
        sql_nanos += started.elapsed().as_nanos();

        report.oracle_pairs += expected.len();
        report.sql_pairs += got.len();
        report.missing += expected.difference(&got).count();
        report.extra += got.difference(&expected).count();

        // The observation's own entity is what it is already attached to; counting it would make
        // every linked observation a "surface hit" and measure nothing.
        let own = observation.knowledge_entity_id;
        let foreign = |ids: &BTreeSet<i32>| -> BTreeSet<i32> {
            ids.iter().copied().filter(|id| Some(*id) != own).collect()
        };
        let surface = foreign(&surface_hits(db, &observation.name).await?.into_iter().collect());
        let word = foreign(&got);
        if !surface.is_empty() {
            report.surface_hits += 1;
        } else if !word.is_empty() {
            report.word_only += 1;
            if report.examples.len() < 12 {
                report.examples.push(observation.name.clone());
            }
        }
    }
    if report.observations > 0 {
        report.oracle_query_ms = oracle_nanos as f64 / 1e6 / report.observations as f64;
        report.sql_query_ms = sql_nanos as f64 / 1e6 / report.observations as f64;
        report.observations_per_illumination = report.observations as f64 / illuminations.len() as f64;
    }
    Ok(report)
}

/// What the folded-surface generation alone would have offered.
async fn surface_hits(db: &DbHandle, name: &str) -> anyhow::Result<Vec<i32>> {
    let folded = normalize::comparable(name);
    if folded.is_empty() {
        return Ok(Vec::new());
    }
    let rows = db
        .conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT knowledge_entity_id FROM knowledge_index WHERE surfaces && $1::varchar[]",
            [vec![folded].into()],
        ))
        .await?;
    Ok(rows
        .iter()
        .map(|row| row.try_get_by_index::<i32>(0))
        .collect::<Result<_, _>>()?)
}
