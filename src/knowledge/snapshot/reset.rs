//! Snapshot the projected graph, then empty it so it can be rebuilt.
//!
//! Observations are untouched — reset only unlinks them. The snapshot exists because the question
//! while tuning the scorer is *what changed between two runs*, and that needs the previous partition
//! to still be queryable after the rebuild.
//!
//! Snapshotting copies rather than renames. Renaming the live tables would drag the foreign key from
//! `user_observed_entities.knowledge_entity_id` along with them — Postgres keeps a constraint pointing at the
//! same table — so the new empty entity table would have nothing referencing it while the archive did,
//! and the fixed index name on the evidence table would then collide when it was recreated.
//! `CREATE TABLE AS SELECT` sidesteps all of it: a frozen artifact needs no constraints, and the live
//! schema is left exactly as it was.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, PaginatorTrait, QueryFilter,
    Statement, TransactionTrait,
};

use super::record::{
    CLAIMS_TABLE, COLLISIONS_TABLE, CORE_TABLE, IDENTITIES_TABLE, MERGES_TABLE, Snapshot,
};

const OBSERVATION_TABLE: &str = "user_observed_entities";
use crate::database;
use crate::model::{
    knowledge_claims as claims, knowledge_collisions as collisions, knowledge_entity as entity,
    knowledge_identity as identities, knowledge_merges as merges,
    user_observed_entity as observation,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Observations whose `knowledge_entity_id` was cleared.
    pub unlinked: u64,
    pub claims: u64,
    pub identities: u64,
    pub merges: u64,
    pub collisions: u64,
    pub entities: u64,
    /// `None` when there was nothing to snapshot.
    pub snapshot: Option<Snapshot>,
    pub snapshot_entities: u64,
    pub snapshot_claims: u64,
    pub snapshot_identities: u64,
    pub snapshot_merges: u64,
    pub snapshot_collisions: u64,
    pub snapshot_members: u64,
}

pub async fn reset(db: &database::DbHandle) -> anyhow::Result<Report> {
    let snapshot = Snapshot::now();
    let report = db
        .conn
        .transaction::<_, Report, anyhow::Error>(|txn| {
            let snapshot = snapshot.clone();
            Box::pin(async move {
                if entity::Entity::find().count(txn).await? == 0 {
                    // Nothing to preserve: do not leave empty archive tables behind.
                    return Ok(Report {
                        unlinked: unlink(txn).await?,
                        ..Default::default()
                    });
                }

                let snapshot_entities = copy(txn, &snapshot.entities(), CORE_TABLE).await?;
                let snapshot_identities =
                    copy(txn, &snapshot.identities(), IDENTITIES_TABLE).await?;
                let snapshot_claims = copy(txn, &snapshot.claims(), CLAIMS_TABLE).await?;
                let snapshot_merges = copy(txn, &snapshot.merges(), MERGES_TABLE).await?;
                let snapshot_collisions =
                    copy(txn, &snapshot.collisions(), COLLISIONS_TABLE).await?;
                let snapshot_members = copy_membership(txn, &snapshot.membership()).await?;

                // Order follows the foreign keys: observations first, then evidence, then entities.
                // Order follows the foreign keys: edges before the identities they cite.
                let unlinked = unlink(txn).await?;
                let claims = remove_claims(txn).await?;
                let collisions = remove_collisions(txn).await?;
                let merges = remove_merges(txn).await?;
                let identities = remove_identities(txn).await?;
                let entities = remove_entities(txn).await?;

                Ok(Report {
                    unlinked,
                    claims,
                    identities,
                    merges,
                    collisions,
                    entities,
                    snapshot: Some(snapshot),
                    snapshot_entities,
                    snapshot_claims,
                    snapshot_identities,
                    snapshot_merges,
                    snapshot_collisions,
                    snapshot_members,
                })
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("reset failed, nothing was changed: {error}"))?;
    Ok(report)
}

/// Names are generated here (digits and a `T`), never user input, so quoting them is sufficient.
async fn copy<C: ConnectionTrait>(conn: &C, snapshot: &str, source: &str) -> anyhow::Result<u64> {
    conn.execute_unprepared(&format!(
        "CREATE TABLE \"{snapshot}\" AS SELECT * FROM \"{source}\""
    ))
    .await?;
    count(conn, snapshot).await
}

async fn copy_membership<C: ConnectionTrait>(conn: &C, snapshot: &str) -> anyhow::Result<u64> {
    conn.execute_unprepared(&format!(
        "CREATE TABLE \"{snapshot}\" AS \
         SELECT id AS observation_id, knowledge_entity_id FROM \"{OBSERVATION_TABLE}\" WHERE knowledge_entity_id IS NOT NULL"
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
        .col_expr(
            observation::Column::KnowledgeEntityId,
            Expr::value(Option::<i32>::None),
        )
        .filter(observation::Column::KnowledgeEntityId.is_not_null())
        .exec(conn)
        .await?;
    Ok(result.rows_affected)
}

async fn remove_identities<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(identities::Entity::delete_many()
        .exec(conn)
        .await?
        .rows_affected)
}

async fn remove_claims<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(claims::Entity::delete_many()
        .exec(conn)
        .await?
        .rows_affected)
}

async fn remove_merges<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(merges::Entity::delete_many()
        .exec(conn)
        .await?
        .rows_affected)
}

async fn remove_collisions<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(collisions::Entity::delete_many()
        .exec(conn)
        .await?
        .rows_affected)
}

async fn remove_entities<C: ConnectionTrait>(conn: &C) -> anyhow::Result<u64> {
    Ok(entity::Entity::delete_many()
        .exec(conn)
        .await?
        .rows_affected)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

    use super::*;
    use crate::test_support::fixtures;

    /// Observation ids grouped by entity, ids of the entities ignored, so two runs can be compared even
    /// though their entity ids differ.
    fn group(rows: Vec<(i32, i32)>) -> Vec<Vec<i32>> {
        let mut groups: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
        for (observation_id, knowledge_entity_id) in rows {
            groups
                .entry(knowledge_entity_id)
                .or_default()
                .push(observation_id);
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
            .filter(observation::Column::KnowledgeEntityId.is_not_null())
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.id, row.knowledge_entity_id.unwrap()))
            .collect();
        group(rows)
    }

    async fn snapshot_partition(db: &database::DbHandle, table: &str) -> Vec<Vec<i32>> {
        let rows = db
            .conn
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT observation_id, knowledge_entity_id FROM \"{table}\""),
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
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        let claimed = entity::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("linking made an entity");
        fixtures::claim(&db, claimed.id, "wikidata", "Q317521", "test", None, None).await;
        let before = live_partition(&db).await;
        assert_eq!(before, vec![vec![1, 2], vec![3]], "{before:?}");

        let report = reset(&db).await.expect("reset should succeed");

        assert_eq!(report.unlinked, 3, "{report:?}");
        assert_eq!(report.entities, 2, "{report:?}");
        assert_eq!(report.snapshot_entities, 2, "{report:?}");
        assert_eq!(report.snapshot_claims, 1, "{report:?}");
        assert_eq!(report.claims, 1, "{report:?}");
        assert_eq!(report.snapshot_members, 3, "{report:?}");
        assert_eq!(
            observation::Entity::find().count(&db.conn).await.unwrap(),
            3,
            "observations must survive a reset"
        );
        assert_eq!(entity::Entity::find().count(&db.conn).await.unwrap(), 0);

        let snapshot = report.snapshot.expect("a snapshot should have been taken");
        assert_eq!(
            snapshot_partition(&db, &snapshot.membership()).await,
            before,
            "the snapshot must reproduce the partition it replaced"
        );

        crate::test_support::link::link_all(&db)
            .await
            .expect("rebuild should succeed");
        assert_eq!(
            live_partition(&db).await,
            before,
            "rebuild must be deterministic"
        );
    }

    #[tokio::test]
    async fn two_resets_keep_two_separate_snapshots() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db).await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        let first = reset(&db).await.expect("reset should succeed");
        crate::test_support::link::link_all(&db)
            .await
            .expect("rebuild should succeed");
        let second = reset(&db).await.expect("second reset should succeed");

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

        let report = reset(&db).await.expect("reset should succeed");

        assert_eq!(report, Report::default());
        assert!(report.snapshot.is_none());
    }
}
