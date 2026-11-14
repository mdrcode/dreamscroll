//! What a run spent, and what the table holds: the two ways this layer is counted.
//!
//! `Stats` is a run: what was asked, what the cache answered, what was fetched. `Tally` is the table:
//! how many rows exist and what became of them. They are different questions over different
//! lifetimes, which is why they are here together and not in `cache` — the cache reads and writes
//! rows, it does not report on them.

use std::sync::atomic::{AtomicUsize, Ordering};

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::transport::Outcome;

/// What one run has spent and found, once it is over.
///
/// The counters are written by any provider at any time and read exactly once, at the end of the
/// run. A named type rather than four loose atomics on the gateway: the report is assembled from one
/// snapshot, and the gateway no longer needs to know what a report is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Requests handed to `answers_for`, including cache hits.
    pub requested: usize,
    /// Answers the cache already had.
    pub cache_hits: usize,
    /// HTTP requests made. One transport call carries a batch of queries and may make more than one
    /// request (a title lookup reads the page, then the items it names), which is why this is not
    /// the same number as the calls the budget was charged for.
    pub fetched: usize,
    /// Queries whose fetch failed.
    pub failed: usize,
}

/// How many rows the cache holds, by what became of them. `expired` counts rows past their expiry,
/// which are not answers any more and are refetchable.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub found: u64,
    pub empty: u64,
    pub failed: u64,
    pub degraded: u64,
    pub expired: u64,
}

/// Count the table, interpreting each row's outcome in exactly one place.
///
/// One grouped query: the counts are the database's job, and the table is large enough that reading
/// every row back to count it was most of what `stats` did.
pub async fn tally(conn: &DatabaseConnection) -> anyhow::Result<Tally> {
    let mut tally = Tally::default();
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT outcome, expires_at <= now() AS expired, count(*)::bigint \
               FROM fetch_cache \
              GROUP BY 1, 2",
        ))
        .await?;
    for row in rows {
        let outcome: String = row.try_get_by_index(0)?;
        let expired: bool = row.try_get_by_index(1)?;
        let count = row.try_get_by_index::<i64>(2)? as u64;
        if expired {
            tally.expired += count;
            continue;
        }
        match Outcome::parse(&outcome) {
            Some(Outcome::Found) => tally.found += count,
            Some(Outcome::Empty) => tally.empty += count,
            Some(Outcome::Failed) => tally.failed += count,
            Some(Outcome::Degraded) => tally.degraded += count,
            None => {}
        }
    }
    Ok(tally)
}

/// The write side of [`Stats`]: one atomic per field, so a provider can record what it did without
/// holding a lock and the run reads a consistent snapshot at the end.
#[derive(Default)]
pub(crate) struct Counters {
    requested: AtomicUsize,
    cache_hits: AtomicUsize,
    fetched: AtomicUsize,
    failed: AtomicUsize,
}

impl Counters {
    pub(crate) fn count_requested(&self, count: usize) {
        self.requested.fetch_add(count, Ordering::Relaxed);
    }

    pub(crate) fn hit(&self, answers: usize) {
        self.cache_hits.fetch_add(answers, Ordering::Relaxed);
    }

    pub(crate) fn fetch(&self, requests: usize) {
        self.fetched.fetch_add(requests, Ordering::Relaxed);
    }

    pub(crate) fn fail(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> Stats {
        Stats {
            requested: self.requested.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            fetched: self.fetched.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Answer, FetchCache};
    use crate::test_support::db_or_skip;

    /// The tally is the one place a stored outcome string becomes a count.
    #[tokio::test]
    async fn the_tally_counts_each_outcome_once() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        for (query, outcome) in [
            ("a", Outcome::Found),
            ("b", Outcome::Empty),
            ("c", Outcome::Failed),
            ("d", Outcome::Degraded),
        ] {
            cache
                .put(
                    "probe",
                    &"kind-a",
                    "v1",
                    query,
                    &Answer::<Vec<String>>::nothing(outcome),
                )
                .await
                .expect("put should succeed");
        }

        let counted = tally(&db.conn).await.expect("tally");
        assert_eq!(
            counted,
            Tally {
                found: 1,
                empty: 1,
                failed: 1,
                degraded: 1,
                expired: 0
            }
        );
    }
}
