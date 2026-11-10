//! Snapshot the projected graph, then empty it so it can be rebuilt.
//!
//! Observations are untouched — reset only unlinks them. The snapshot exists because the question
//! while tuning the scorer is *what changed between two runs*, and that needs the previous partition
//! to still be queryable after the rebuild.
//!
//! Snapshotting copies rather than renames; [`super::record`] explains why.

use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, PaginatorTrait, QueryFilter, Set,
    Statement, TransactionTrait,
};

use super::record::{
    CLAIMS_TABLE, CORE_TABLE, IDENTITIES_TABLE, PROXIMITIES_TABLE, Snapshot,
};

use crate::database;
use crate::model::{
    knowledge_blocking_index as blocking, knowledge_claims as claims,
    knowledge_entity as entity, knowledge_generations as generation,
    knowledge_identity as identities, knowledge_proximities as proximities,
    knowledge_verdicts as verdicts, user_observed_entity as observation,
};

const OBSERVATION_TABLE: &str = "user_observed_entities";

/// What one reset did. Merges are absent by construction: `knowledge_verdicts` is never cleared, so
/// the count a generation recorded belongs to its register row, not to this report.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Observations whose `knowledge_entity_id` was cleared.
    pub unlinked: u64,
    pub claims: u64,
    pub proximities: u64,
    pub entities: u64,
    /// `None` when there was nothing to snapshot.
    pub snapshot: Option<Snapshot>,
    pub snapshot_entities: u64,
    pub snapshot_claims: u64,
    pub snapshot_identities: u64,
    pub snapshot_proximities: u64,
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
                    let unlinked = unlink(txn).await?;
                    supersede_generation(txn, None, 0, 0, 0, 0, 0).await?;
                    return Ok(Report {
                        unlinked,
                        ..Default::default()
                    });
                }

                let snapshot_entities = copy(txn, &snapshot.entities(), CORE_TABLE).await?;
                let snapshot_identities =
                    copy(txn, &snapshot.identities(), IDENTITIES_TABLE).await?;
                let snapshot_claims = copy(txn, &snapshot.claims(), CLAIMS_TABLE).await?;
                let snapshot_proximities =
                    copy(txn, &snapshot.proximities(), PROXIMITIES_TABLE).await?;
                let snapshot_members = copy_membership(txn, &snapshot.membership()).await?;

                // Order follows the foreign keys: edges before the identities they cite, entities
                // last because everything points at them.
                let unlinked = unlink(txn).await?;
                // Derived from the entities themselves: gone before them, back with them.
                blocking::Entity::delete_many().exec(txn).await?;
                let claims = claims::Entity::delete_many().exec(txn).await?.rows_affected;
                let proximities = proximities::Entity::delete_many().exec(txn).await?.rows_affected;
                identities::Entity::delete_many().exec(txn).await?;
                let entities = entity::Entity::delete_many().exec(txn).await?.rows_affected;

                supersede_generation(
                    txn,
                    Some(&snapshot.suffix),
                    entities,
                    snapshot_identities,
                    claims,
                    proximities,
                    snapshot_members,
                )
                .await?;

                Ok(Report {
                    unlinked,
                    claims,
                    proximities,
                    entities,
                    snapshot: Some(snapshot),
                    snapshot_entities,
                    snapshot_claims,
                    snapshot_identities,
                    snapshot_proximities,
                    snapshot_members,
                })
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("reset failed, nothing was changed: {error}"))?;
    Ok(report)
}

/// Close the generation the graph belonged to and open the next.
///
/// A generation is one build cycle, and reset is the only snapshotter — it empties as it copies — so
/// this row is also the snapshot's register: at most one snapshot belongs to a generation.
async fn supersede_generation<C: ConnectionTrait>(
    conn: &C,
    snapshot: Option<&str>,
    entities: u64,
    identities: u64,
    claims: u64,
    proximities: u64,
    members: u64,
) -> anyhow::Result<()> {
    let current = crate::knowledge::graph::current_generation(conn).await?;
    // Verdicts are never deleted, so the register records how many merges the generation held, not
    // how many reset removed — which is none.
    let merges = verdicts::Entity::find()
        .filter(verdicts::Column::Generation.eq(current))
        .filter(verdicts::Column::Kind.eq(verdicts::Kind::Merge))
        .count(conn)
        .await?;
    let mut conflict = OnConflict::column(generation::Column::Id);
    conflict.do_nothing();
    generation::Entity::insert(generation::ActiveModel {
        id: Set(current),
        ..Default::default()
    })
    .on_conflict(conflict)
    .exec_without_returning(conn)
    .await?;

    generation::Entity::update_many()
        .col_expr(
            generation::Column::SupersededAt,
            Expr::current_timestamp(),
        )
        .col_expr(
            generation::Column::Snapshot,
            Expr::value(snapshot.map(str::to_string)),
        )
        .col_expr(
            generation::Column::Entities,
            Expr::value(Some(entities as i32)),
        )
        .col_expr(
            generation::Column::Identities,
            Expr::value(Some(identities as i32)),
        )
        .col_expr(generation::Column::Claims, Expr::value(Some(claims as i32)))
        .col_expr(generation::Column::Merges, Expr::value(Some(merges as i32)))
        .col_expr(
            generation::Column::Proximities,
            Expr::value(Some(proximities as i32)),
        )
        .col_expr(
            generation::Column::Members,
            Expr::value(Some(members as i32)),
        )
        .filter(generation::Column::Id.eq(current))
        .exec(conn)
        .await?;

    let mut conflict = OnConflict::column(generation::Column::Id);
    conflict.do_nothing();
    generation::Entity::insert(generation::ActiveModel {
        id: Set(current + 1),
        ..Default::default()
    })
    .on_conflict(conflict)
    .exec_without_returning(conn)
    .await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use super::*;
    use crate::test_support::fixtures;
    use crate::test_support::db_or_skip;

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

    /// `(origin, members)` per live entity, ordered — two runs compared without their ids.
    async fn origins(db: &database::DbHandle) -> Vec<(i32, Vec<i32>)> {
        let mut groups = Vec::new();
        for entity in entity::Entity::find().all(&db.conn).await.unwrap() {
            let mut members: Vec<i32> = observation::Entity::find()
                .filter(observation::Column::KnowledgeEntityId.eq(entity.id))
                .all(&db.conn)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.id)
                .collect();
            members.sort_unstable();
            groups.push((
                entity.origin_observation_id.expect("linking set an origin"),
                members,
            ));
        }
        groups.sort();
        groups
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
            let capture = fixtures::Capture {
                user_id,
                capture_id,
                illumination_id,
            };
            fixtures::observe(db, &capture, 0, name, None, "book", None, None).await;
        }
    }

    #[tokio::test]
    async fn reset_preserves_the_partition_and_empties_the_graph() {
        db_or_skip!(db);
        seed(&db).await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        let claimed = entity::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("linking made an entity");
        fixtures::claim(&db, claimed.id, "wikidata", "Q317521", "test", None, None, crate::model::knowledge_claims::ClaimBasis::Text).await;
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

    }

    /// The key a person's decisions are logged under has to come back with the group: entity ids are
    /// reallocated by a rebuild, origins are not. This is what makes `verdicts replay` possible, and
    /// it holds only while linking is ordered (illumination, then position).
    #[tokio::test]
    async fn a_rebuild_gives_every_group_back_its_origin() {
        db_or_skip!(db);
        seed(&db).await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        let before = origins(&db).await;
        assert_eq!(before.len(), 2, "{before:?}");

        reset(&db).await.expect("reset should succeed");
        crate::test_support::link::link_all(&db)
            .await
            .expect("rebuild should succeed");

        assert_eq!(origins(&db).await, before, "each group kept its origin");
    }

    #[tokio::test]
    async fn two_resets_keep_two_separate_snapshots() {
        db_or_skip!(db);
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
        db_or_skip!(db);

        let report = reset(&db).await.expect("reset should succeed");

        assert_eq!(report, Report::default());
        assert!(report.snapshot.is_none());
    }

    /// The register says how many merges a generation recorded. Verdicts are never deleted, so a
    /// reset cannot count what it removed — it counts what the generation held.
    #[tokio::test]
    async fn the_closed_generation_records_its_merges() {
        db_or_skip!(db);
        let store = crate::knowledge::KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        store
            .merge(
                loser,
                winner,
                crate::knowledge::verdicts::Cause::Repair,
            )
            .await
            .unwrap();
        let generation = crate::knowledge::graph::current_generation(&db.conn)
            .await
            .unwrap();

        reset(&db).await.expect("reset should succeed");

        let row = crate::model::knowledge_generations::Entity::find_by_id(generation)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the register keeps the closed generation");
        assert_eq!(row.merges, Some(1), "{row:?}");
    }
}
