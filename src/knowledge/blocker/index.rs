//! `knowledge_blocking_index`: the folded forms a blocker matches on.
//!
//! A function of `name` and `alt_names` and nothing else. Written by `graph::project`
//! whenever an entity's derived columns change, forgotten when the entity stops being live, and
//! rebuilt wholesale when the key rule changes — `built_with` records which rule produced a row,
//! so a stale index is countable instead of being discovered later as a recall bug.
//!
//! See AGENTS.md §Derived Data for why the version is a behavioural fingerprint rather than a
//! hand-bumped number, and `plan/knowledge-blocking.md` §Storage for why this is a table.

use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement};

use crate::database::DbHandle;
use crate::knowledge::normalize;
use crate::model;

/// One row per live entity: the folded surfaces, and the same text for full search.
const UPSERT: &str = "
INSERT INTO knowledge_blocking_index (knowledge_entity_id, surfaces, word_keys, text, built_with)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (knowledge_entity_id) DO UPDATE
   SET surfaces = EXCLUDED.surfaces,
       word_keys = EXCLUDED.word_keys,
       text = EXCLUDED.text,
       built_with = EXCLUDED.built_with,
       built_at = now()";

/// The two generations the index stores, in the entity's surface order.
pub(super) struct Keys {
    /// Folded forms: an exact match on one of these.
    pub surfaces: Vec<String>,
    /// Word-order-insensitive forms: `Train Dreams` matches `Dreams Train`.
    pub word_keys: Vec<String>,
}

/// Every surface the entity wears, in the order it wears them.
///
/// `alt_names` already carries the member names, the platform handle and the display name, so this
/// covers what the blocker can be asked about without a second pass over the members.
pub(super) fn keys(entity: &model::knowledge_entity::Model) -> Keys {
    let mut seen = std::collections::BTreeSet::new();
    let surfaces: Vec<String> = entity
        .surfaces()
        .map(normalize::comparable)
        .filter(|key| !key.is_empty())
        .filter(|key| seen.insert(key.clone()))
        .collect();
    Keys {
        // A surface with no words (`...` folds to `..`) has no word generation. Storing the empty
        // string would make `&&` match it against every other empty one.
        word_keys: surfaces
            .iter()
            .map(|surface| normalize::word_key(surface))
            .filter(|key| !key.is_empty())
            .collect(),
        surfaces,
    }
}

/// Upsert one entity's row. `project` calls this, so it and [`rebuild`] are the only writers.
pub(crate) async fn write<C: ConnectionTrait>(
    conn: &C,
    entity: &model::knowledge_entity::Model,
) -> anyhow::Result<()> {
    // A tombstone has no retrieval material. `project` can be handed one — `identity veto` names an
    // id a merge may have folded since — and re-stamping it would leave a current-fingerprint row
    // that `check` cannot see.
    if entity.merged_into_id.is_some() {
        return forget(conn, entity.id).await;
    }
    let Keys {
        surfaces,
        word_keys,
    } = keys(entity);
    let text = surfaces.join(" ");
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        UPSERT,
        [
            entity.id.into(),
            surfaces.into(),
            word_keys.into(),
            text.into(),
            normalize::norm_fingerprint().into(),
        ],
    ))
    .await?;
    Ok(())
}

/// Drop an entity's row: it was tombstoned by a merge, or the generation was emptied.
pub(crate) async fn forget<C: ConnectionTrait>(conn: &C, knowledge_entity_id: i32) -> anyhow::Result<()> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM knowledge_blocking_index WHERE knowledge_entity_id = $1",
        [knowledge_entity_id.into()],
    ))
    .await?;
    Ok(())
}

/// What the index looks like against the rule that should have produced it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Rows present.
    pub indexed: usize,
    /// Rows whose `built_with` is not the current fingerprint — built by an older rule.
    pub stale: usize,
    /// Live entities with no row at all: a writer was skipped, or a rebuild was interrupted.
    pub missing: usize,
    /// Rows for an entity that is absent or tombstoned: retrieval material with nothing to retrieve.
    pub orphaned: usize,
}

impl Report {
    pub fn is_current(&self) -> bool {
        self.stale == 0 && self.missing == 0 && self.orphaned == 0
    }
}

/// Recompute every live entity's row from its columns.
///
/// A rebuild, not a migration: the table is emptied and refilled, and nothing else is touched.
pub async fn rebuild(db: &DbHandle) -> anyhow::Result<usize> {
    db.conn
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "DELETE FROM knowledge_blocking_index",
        ))
        .await?;
    let entities = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
        .all(&db.conn)
        .await?;
    for entity in &entities {
        write(&db.conn, entity).await?;
    }
    Ok(entities.len())
}

/// How many rows disagree with the current fold, and how many entities have no row.
pub async fn check(db: &DbHandle) -> anyhow::Result<Report> {
    let count = |sql: &'static str, values: Vec<sea_orm::Value>| {
        let conn = &db.conn;
        async move {
            let row = conn
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    sql,
                    values,
                ))
                .await?;
            anyhow::Ok(row.map(|row| row.try_get_by_index::<i64>(0)).transpose()?.unwrap_or(0))
        }
    };
    Ok(Report {
        indexed: count("SELECT count(*) FROM knowledge_blocking_index", vec![]).await? as usize,
        stale: count(
            "SELECT count(*) FROM knowledge_blocking_index WHERE built_with <> $1",
            vec![normalize::norm_fingerprint().into()],
        )
        .await? as usize,
        missing: count(
            "SELECT count(*) FROM knowledge_entities e \
                  LEFT JOIN knowledge_blocking_index i ON i.knowledge_entity_id = e.id \
              WHERE e.merged_into_id IS NULL AND i.knowledge_entity_id IS NULL",
            vec![],
        )
        .await? as usize,
        orphaned: count(
            "SELECT count(*) FROM knowledge_blocking_index i \
                  LEFT JOIN knowledge_entities e ON e.id = i.knowledge_entity_id \
              WHERE e.id IS NULL OR e.merged_into_id IS NOT NULL",
            vec![],
        )
        .await? as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{db_or_skip, fixtures};

    /// `project` writes the row, and what it writes is the fold of the entity's surfaces.
    #[tokio::test]
    async fn projecting_an_entity_writes_its_keys() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        // The fixture inserts the row directly; `project` is the writer under test.
        crate::knowledge::KnowledgeGraph::new(&db).update_projection(id).await.unwrap();
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("entity");

        let row = model::knowledge_blocking_index::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("project must write the index row");
        assert_eq!(row.surfaces, keys(&entity).surfaces);
        assert_eq!(row.word_keys, keys(&entity).word_keys);
        assert_eq!(row.built_with, normalize::norm_fingerprint());
        assert!(row.surfaces.contains(&"train dreams".to_string()));
        assert!(row.word_keys.contains(&"dreams train".to_string()));
        assert_eq!(
            check(&db).await.unwrap(),
            Report {
                indexed: 1,
                stale: 0,
                missing: 0,
                orphaned: 0
            }
        );
    }

    /// A row built by an older fold is stale, and `rebuild` is what fixes it.
    #[tokio::test]
    async fn a_row_from_another_fold_is_counted_and_rebuildable() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::KnowledgeGraph::new(&db).update_projection(id).await.unwrap();
        db.conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE knowledge_blocking_index SET built_with = 'fold-0000000000000000' WHERE knowledge_entity_id = $1",
                [id.into()],
            ))
            .await
            .unwrap();

        let before = check(&db).await.unwrap();
        assert_eq!(before.stale, 1, "{before:?}");

        assert_eq!(rebuild(&db).await.unwrap(), 1);
        let after = check(&db).await.unwrap();
        assert!(after.is_current(), "{after:?}");
    }

    /// A merge tombstones the loser and deletes its row: a tombstone has no surfaces to block on.
    /// The survivor's row then carries both sides' surfaces, which is how the merged name keeps
    /// resolving.
    #[tokio::test]
    async fn a_merge_drops_the_losers_row_and_widens_the_winners() {
        db_or_skip!(db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Elon Musk").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Elon").await;
        let store = crate::knowledge::KnowledgeGraph::new(&db);
        store.update_projection(winner).await.unwrap();
        store.update_projection(loser).await.unwrap();

        store
            .merge(loser, winner, crate::knowledge::verdicts::Cause::Repair)
            .await
            .unwrap();

        assert!(
            model::knowledge_blocking_index::Entity::find_by_id(loser)
                .one(&db.conn)
                .await
                .unwrap()
                .is_none(),
            "the tombstone keeps no retrieval material"
        );
        let survivor = model::knowledge_blocking_index::Entity::find_by_id(winner)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the winner keeps its row");
        assert!(
            survivor.surfaces.contains(&"elon".to_string()),
            "the folded-away name is still reachable: {:?}",
            survivor.surfaces
        );
        assert!(check(&db).await.unwrap().is_current());
    }

    /// The blocker must not offer a tombstone, even if a stale row somehow survived it.
    #[tokio::test]
    async fn a_tombstoned_entity_is_never_offered() {
        db_or_skip!(db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let store = crate::knowledge::KnowledgeGraph::new(&db);
        store.update_projection(winner).await.unwrap();
        store.update_projection(loser).await.unwrap();
        store
            .merge(loser, winner, crate::knowledge::verdicts::Cause::Repair)
            .await
            .unwrap();

        let offered = crate::knowledge::blocker::sql::candidate_ids(
            &db,
            &fixtures::stub_observation(1, "Train Dreams", "book"),
        )
        .await
        .unwrap();
        assert_eq!(offered, vec![winner], "only the live side is a candidate");
    }

    /// A live entity with no row is reported as missing, which is a different failure from stale.
    #[tokio::test]
    async fn a_live_entity_without_a_row_is_missing() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::KnowledgeGraph::new(&db).update_projection(id).await.unwrap();
        forget(&db.conn, id).await.unwrap();
        let report = check(&db).await.unwrap();
        assert_eq!(report.missing, 1, "{report:?}");
        assert_eq!(report.stale, 0, "{report:?}");
    }

    /// A surface with no words still gets a surface key, and no word key: the empty string would
    /// overlap every other empty one through `&&`.
    #[tokio::test]
    async fn a_wordless_surface_gets_no_word_key() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "...").await;
        crate::knowledge::KnowledgeGraph::new(&db)
            .update_projection(id)
            .await
            .unwrap();

        let row = model::knowledge_blocking_index::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("project writes the row");
        assert_eq!(
            row.surfaces,
            vec!["..".to_string()],
            "the surface is still keyed"
        );
        assert!(row.word_keys.is_empty(), "{:?}", row.word_keys);
    }

    /// `project` can be handed a tombstone: a caller may hold an id that a merge folded first. A row
    /// for it would outlive its liveness and `check` would still call the index current.
    #[tokio::test]
    async fn projecting_a_tombstone_writes_no_row() {
        db_or_skip!(db);
        let store = crate::knowledge::KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        store.update_projection(winner).await.unwrap();
        store.update_projection(loser).await.unwrap();
        store
            .merge(
                loser,
                winner,
                crate::knowledge::verdicts::Cause::Repair,
            )
            .await
            .unwrap();

        let tombstone = store.get(loser).await.unwrap();
        assert!(tombstone.merged_into_id.is_some());
        store.update_projection(loser).await.unwrap();

        assert!(
            model::knowledge_blocking_index::Entity::find_by_id(loser)
                .one(&db.conn)
                .await
                .unwrap()
                .is_none(),
            "a tombstone keeps no retrieval material"
        );
        assert!(check(&db).await.unwrap().is_current());
    }

    /// A row can outlive its entity — an older build wrote it, or a crash parted it from its
    /// tombstone. `check` counts it, or `is_current` promises a table it has not verified.
    #[tokio::test]
    async fn an_orphaned_row_is_counted() {
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

        // The state a writer that predates the guard leaves behind.
        db.conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO knowledge_blocking_index \
                   (knowledge_entity_id, surfaces, word_keys, text, built_with) \
                 VALUES ($1, ARRAY['train dreams'], ARRAY['dreams train'], 'train dreams', $2)",
                [loser.into(), normalize::norm_fingerprint().into()],
            ))
            .await
            .unwrap();

        let report = check(&db).await.unwrap();
        assert_eq!(report.orphaned, 1, "{report:?}");
        assert_eq!(report.missing, 0, "{report:?}");
        assert!(!report.is_current(), "{report:?}");
    }
}
