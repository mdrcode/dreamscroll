//! Snapshot the projected graph, then empty it so it can be rebuilt.
//!
//! Observations are untouched — reset only unlinks them. The snapshot exists because the question
//! while tuning the scorer is *what changed between two runs*, and that needs the previous partition
//! to still be queryable after the rebuild.
//!
//! Snapshotting copies rather than renames. Renaming the live tables would drag the foreign key from
//! `user_knowledge_entities.core_id` along with them — Postgres keeps a constraint pointing at the
//! same table — so the new empty core table would have nothing referencing it while the archive did,
//! and the fixed index name on the evidence table would then collide when it was recreated.
//! `CREATE TABLE AS SELECT` sidesteps all of it: a frozen artifact needs no constraints, and the live
//! schema is left exactly as it was.

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, PaginatorTrait, QueryFilter,
    Statement, TransactionTrait,
};

use crate::database;
use crate::model::{
    core_knowledge_entity as core, core_knowledge_evidence as evidence,
    user_knowledge_entity as observation,
};

const CORE_TABLE: &str = "core_knowledge_entities";
const EVIDENCE_TABLE: &str = "core_knowledge_evidence";
const OBSERVATION_TABLE: &str = "user_knowledge_entities";

/// A frozen copy of one run, named by the moment it was taken so two runs sort chronologically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub suffix: String,
}

impl Snapshot {
    pub fn now() -> Self {
        Self {
            suffix: Utc::now().format("%Y%m%dT%H%M%S%3f").to_string(),
        }
    }

    pub fn cores(&self) -> String {
        format!("{CORE_TABLE}__{}", self.suffix)
    }

    pub fn evidence(&self) -> String {
        format!("{EVIDENCE_TABLE}__{}", self.suffix)
    }

    /// Which observations belonged to which core — the only record of that is
    /// `user_knowledge_entities.core_id`, so snapshotting the cores alone would keep the referents
    /// and lose the membership. One row per linked observation.
    pub fn membership(&self) -> String {
        format!("core_knowledge_membership__{}", self.suffix)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Observations whose `core_id` was cleared.
    pub unlinked: u64,
    pub evidence: u64,
    pub cores: u64,
    /// `None` when there was nothing to snapshot.
    pub snapshot: Option<Snapshot>,
    pub snapshot_cores: u64,
    pub snapshot_evidence: u64,
    pub snapshot_members: u64,
}

pub async fn reset_core(db: &database::DbHandle) -> anyhow::Result<Report> {
    let snapshot = Snapshot::now();
    let report = db
        .conn
        .transaction::<_, Report, anyhow::Error>(|txn| {
            let snapshot = snapshot.clone();
            Box::pin(async move {
                if core::Entity::find().count(txn).await? == 0 {
                    // Nothing to preserve: do not leave empty archive tables behind.
                    return Ok(Report {
                        unlinked: unlink(txn).await?,
                        ..Default::default()
                    });
                }

                let snapshot_cores = copy(txn, &snapshot.cores(), CORE_TABLE).await?;
                let snapshot_evidence = copy(txn, &snapshot.evidence(), EVIDENCE_TABLE).await?;
                let snapshot_members = copy_membership(txn, &snapshot.membership()).await?;

                // Order follows the foreign keys: observations first, then evidence, then cores.
                let unlinked = unlink(txn).await?;
                let evidence = remove_evidence(txn).await?;
                let cores = remove_cores(txn).await?;

                Ok(Report {
                    unlinked,
                    evidence,
                    cores,
                    snapshot: Some(snapshot),
                    snapshot_cores,
                    snapshot_evidence,
                    snapshot_members,
                })
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("reset failed, nothing was changed: {error}"))?;
    Ok(report)
}

/// Names are generated here (digits and a `T`), never user input, so quoting them is sufficient.
async fn copy<C: ConnectionTrait>(
    conn: &C,
    snapshot: &str,
    source: &str,
) -> anyhow::Result<u64> {
    conn.execute_unprepared(&format!(
        "CREATE TABLE \"{snapshot}\" AS SELECT * FROM \"{source}\""
    ))
    .await?;
    count(conn, snapshot).await
}

async fn copy_membership<C: ConnectionTrait>(conn: &C, snapshot: &str) -> anyhow::Result<u64> {
    conn.execute_unprepared(&format!(
        "CREATE TABLE \"{snapshot}\" AS \
         SELECT id AS observation_id, core_id FROM \"{OBSERVATION_TABLE}\" WHERE core_id IS NOT NULL"
    ))
    .await?;
    count(conn, snapshot).await
}

async fn count<C: ConnectionTrait>(conn: &C, table: &str) -> anyhow::Result<u64> {
    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT count(*) FROM \"{table}\""),
        ))
        .await?
        .expect("count always returns one row");
    Ok(row.try_get_by_index::<i64>(0)? as u64)
}

async fn unlink<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    let result = observation::Entity::update_many()
        .col_expr(observation::Column::CoreId, Expr::value(Option::<i32>::None))
        .filter(observation::Column::CoreId.is_not_null())
        .exec(conn)
        .await?;
    Ok(result.rows_affected)
}

async fn remove_evidence<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(evidence::Entity::delete_many()
        .exec(conn)
        .await?
        .rows_affected)
}

async fn remove_cores<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(core::Entity::delete_many().exec(conn).await?.rows_affected)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

    use super::*;
    use crate::knowledge::link;
    use crate::test_support::fixtures;

    /// Observation ids grouped by core, ids of the cores ignored, so two runs can be compared even
    /// though their core ids differ.
    fn group(rows: Vec<(i32, i32)>) -> Vec<Vec<i32>> {
        let mut groups: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
        for (observation_id, core_id) in rows {
            groups.entry(core_id).or_default().push(observation_id);
        }
        let mut partition: Vec<Vec<i32>> = groups.into_values().collect();
        for group in &mut partition {
            group.sort_unstable();
        }
        partition.sort();
        partition
    }

    async fn live_partition(db: &database::DbHandle) -> Vec<Vec<i32>> {
        let rows = observation::Entity::find()
            .filter(observation::Column::CoreId.is_not_null())
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.id, row.core_id.unwrap()))
            .collect();
        group(rows)
    }

    async fn snapshot_partition(db: &database::DbHandle, table: &str) -> Vec<Vec<i32>> {
        let rows = db
            .conn
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT observation_id, core_id FROM \"{table}\""),
            ))
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.try_get_by_index::<i32>(0).unwrap(),
                    row.try_get_by_index::<i32>(1).unwrap(),
                )
            })
            .collect();
        group(rows)
    }

    async fn seed(db: &database::DbHandle) {
        let user_id = fixtures::user(db, "reset").await;
        for name in ["Train Dreams", "train dreams", "Gravity's Rainbow"] {
            let capture_id = fixtures::capture(db, user_id).await;
            let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
            observation::Entity::insert(observation::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(0),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn reset_preserves_the_partition_and_empties_the_graph() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db).await;
        link::build_core(&db).await.expect("build should succeed");
        let before = live_partition(&db).await;
        assert_eq!(before, vec![vec![1, 2], vec![3]], "{before:?}");

        let report = reset_core(&db).await.expect("reset should succeed");

        assert_eq!(report.unlinked, 3, "{report:?}");
        assert_eq!(report.cores, 2, "{report:?}");
        assert_eq!(report.snapshot_cores, 2, "{report:?}");
        assert_eq!(report.snapshot_members, 3, "{report:?}");
        assert_eq!(
            observation::Entity::find().count(&db.conn).await.unwrap(),
            3,
            "observations must survive a reset"
        );
        assert_eq!(core::Entity::find().count(&db.conn).await.unwrap(), 0);
        assert_eq!(evidence::Entity::find().count(&db.conn).await.unwrap(), 0);

        let snapshot = report.snapshot.expect("a snapshot should have been taken");
        assert_eq!(
            snapshot_partition(&db, &snapshot.membership()).await,
            before,
            "the snapshot must reproduce the partition it replaced"
        );

        link::build_core(&db).await.expect("rebuild should succeed");
        assert_eq!(live_partition(&db).await, before, "rebuild must be deterministic");
    }

    #[tokio::test]
    async fn two_resets_keep_two_separate_snapshots() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db).await;

        link::build_core(&db).await.expect("build should succeed");
        let first = reset_core(&db).await.expect("reset should succeed");
        link::build_core(&db).await.expect("rebuild should succeed");
        let second = reset_core(&db).await.expect("second reset should succeed");

        let (first, second) = (
            first.snapshot.expect("first snapshot"),
            second.snapshot.expect("second snapshot"),
        );
        assert_ne!(first.suffix, second.suffix, "each run keeps its own name");
        assert_eq!(count(&db.conn, &first.membership()).await.unwrap(), 3);
        assert_eq!(count(&db.conn, &second.membership()).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn resetting_an_empty_graph_creates_no_snapshot() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let report = reset_core(&db).await.expect("reset should succeed");

        assert_eq!(report, Report::default());
        assert!(report.snapshot.is_none());
    }
}
