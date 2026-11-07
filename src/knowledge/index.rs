//! `knowledge_index`: the folded forms a blocker matches on.
//!
//! A function of `name` and `alt_names` and nothing else. Written by [`crate::knowledge::project`]
//! whenever an entity's derived columns change, forgotten when the entity stops being live, and
//! rebuilt wholesale when the fold changes — `folded_with` records which fold produced a row, so a
//! stale index is countable instead of being discovered later as a recall bug.
//!
//! See AGENTS.md §Derived Data for why the version is a behavioural fingerprint rather than a
//! hand-bumped number, and `plan/knowledge-blocking.md` §Storage for why this is a table.

use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement};

use crate::database::DbHandle;
use crate::knowledge::normalize;
use crate::model;

/// One row per live entity: the folded surfaces, and the same text for full search.
const UPSERT: &str = "
INSERT INTO knowledge_index (knowledge_entity_id, surfaces, text, folded_with)
VALUES ($1, $2, $3, $4)
ON CONFLICT (knowledge_entity_id) DO UPDATE
   SET surfaces = EXCLUDED.surfaces,
       text = EXCLUDED.text,
       folded_with = EXCLUDED.folded_with,
       built_at = now()";

/// Every folded surface the entity wears, deduped and in the order it wears them.
///
/// `alt_names` already carries the member names, the platform handle and the display name, so this
/// covers what the blocker can be asked about without a second pass over the members.
pub(super) fn keys(entity: &model::knowledge_entity::Model) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    entity
        .surfaces()
        .map(normalize::comparable)
        .filter(|key| !key.is_empty())
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

/// Upsert one entity's row. `project` calls this, so it and [`rebuild`] are the only writers.
pub(super) async fn write<C: ConnectionTrait>(
    conn: &C,
    entity: &model::knowledge_entity::Model,
) -> anyhow::Result<()> {
    let surfaces = keys(entity);
    let text = surfaces.join(" ");
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        UPSERT,
        [
            entity.id.into(),
            surfaces.into(),
            text.into(),
            normalize::fingerprint().into(),
        ],
    ))
    .await?;
    Ok(())
}

/// Drop an entity's row: it was tombstoned by a merge, or the generation was emptied.
pub(super) async fn forget<C: ConnectionTrait>(conn: &C, knowledge_entity_id: i32) -> anyhow::Result<()> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM knowledge_index WHERE knowledge_entity_id = $1",
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
    /// Rows whose `folded_with` is not the current fingerprint — built by an older fold.
    pub stale: usize,
    /// Live entities with no row at all: a writer was skipped, or a rebuild was interrupted.
    pub missing: usize,
}

impl Report {
    pub fn is_current(&self) -> bool {
        self.stale == 0 && self.missing == 0
    }
}

/// Recompute every live entity's row from its columns.
///
/// A rebuild, not a migration: the table is emptied and refilled, and nothing else is touched.
pub async fn rebuild(db: &DbHandle) -> anyhow::Result<usize> {
    db.conn
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "DELETE FROM knowledge_index",
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
        indexed: count("SELECT count(*) FROM knowledge_index", vec![]).await? as usize,
        stale: count(
            "SELECT count(*) FROM knowledge_index WHERE folded_with <> $1",
            vec![normalize::fingerprint().into()],
        )
        .await? as usize,
        missing: count(
            "SELECT count(*) FROM knowledge_entities e \
                  LEFT JOIN knowledge_index i ON i.knowledge_entity_id = e.id \
              WHERE e.merged_into_id IS NULL AND i.knowledge_entity_id IS NULL",
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
        crate::knowledge::Knowledge::new(&db).update_projection(id).await.unwrap();
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("entity");

        let row = model::knowledge_index::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("project must write the index row");
        assert_eq!(row.surfaces, keys(&entity));
        assert_eq!(row.folded_with, normalize::fingerprint());
        assert!(row.surfaces.contains(&"train dreams".to_string()));
        assert_eq!(check(&db).await.unwrap(), Report { indexed: 1, stale: 0, missing: 0 });
    }

    /// A row built by an older fold is stale, and `rebuild` is what fixes it.
    #[tokio::test]
    async fn a_row_from_another_fold_is_counted_and_rebuildable() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::Knowledge::new(&db).update_projection(id).await.unwrap();
        db.conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE knowledge_index SET folded_with = 'fold-0000000000000000' WHERE knowledge_entity_id = $1",
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

    /// A live entity with no row is reported as missing, which is a different failure from stale.
    #[tokio::test]
    async fn a_live_entity_without_a_row_is_missing() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::Knowledge::new(&db).update_projection(id).await.unwrap();
        forget(&db.conn, id).await.unwrap();
        let report = check(&db).await.unwrap();
        assert_eq!(report.missing, 1, "{report:?}");
        assert_eq!(report.stale, 0, "{report:?}");
    }
}
