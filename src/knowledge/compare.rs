//! Diff two snapshots' membership: what the last change to the linker actually did.
//!
//! Core ids are not comparable across runs — they come from a sequence that keeps advancing — so the
//! comparison is over *groupings*: which observations ended up in the same core, expressed by
//! observation id, which is stable because reset never touches observations.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::database;
use super::reset::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One group in the earlier run became several.
    Split,
    /// Several groups in the earlier run became one.
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub kind: Kind,
    /// Label of the earlier core (split) or the later core (merged).
    pub label: String,
    /// Members in the earlier run for a split, in the later run for a merge.
    pub members: usize,
    /// Groups on the other side.
    pub into: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub observations: usize,
    /// Observations present in one snapshot but not the other. Normally zero: reset does not touch
    /// observations, so a non-zero count means the corpus changed between runs.
    pub only_in_before: usize,
    pub only_in_after: usize,
    pub cores_before: usize,
    pub cores_after: usize,
    /// Groups whose membership is identical in both runs.
    pub unchanged: usize,
    pub split: usize,
    pub merged: usize,
    /// Observations whose set of group-mates changed.
    pub moved: usize,
    pub examples: Vec<Example>,
}

impl Report {
    pub fn identical(&self) -> bool {
        self.split == 0 && self.merged == 0 && self.moved == 0 && self.only_in_before == 0
            && self.only_in_after == 0
    }
}

pub async fn compare(
    db: &database::DbHandle,
    before: &Snapshot,
    after: &Snapshot,
) -> anyhow::Result<Report> {
    let before_map = membership(db, &before.membership()).await?;
    let after_map = membership(db, &after.membership()).await?;
    let before_labels = labels(db, &before.cores()).await?;
    let after_labels = labels(db, &after.cores()).await?;

    let before_groups = groups(&before_map);
    let after_groups = groups(&after_map);

    let mut report = Report {
        observations: before_map.len().max(after_map.len()),
        only_in_before: before_map.keys().filter(|id| !after_map.contains_key(id)).count(),
        only_in_after: after_map.keys().filter(|id| !before_map.contains_key(id)).count(),
        cores_before: before_groups.len(),
        cores_after: after_groups.len(),
        ..Default::default()
    };

    // A group is unchanged when its membership appears identically on the other side.
    let before_sets: BTreeSet<Vec<i32>> = before_groups.values().cloned().collect();
    let after_sets: BTreeSet<Vec<i32>> = after_groups.values().cloned().collect();
    report.unchanged = before_sets.intersection(&after_sets).count();

    // An observation moved when its set of group-mates differs, which is exactly what a merge or a
    // split changes.
    let before_room = group_mates(&before_groups);
    let after_room = group_mates(&after_groups);
    report.moved = before_map
        .keys()
        .filter(|id| before_room.get(id) != after_room.get(*id))
        .count();

    let mut examples = Vec::new();
    for (core, members) in &before_groups {
        let into: BTreeSet<i32> = members
            .iter()
            .filter_map(|id| after_map.get(id))
            .copied()
            .collect();
        if into.len() > 1 {
            report.split += 1;
            examples.push(Example {
                kind: Kind::Split,
                label: before_labels.get(core).cloned().unwrap_or_default(),
                members: members.len(),
                into: into.len(),
            });
        }
    }
    for (core, members) in &after_groups {
        let from: BTreeSet<i32> = members
            .iter()
            .filter_map(|id| before_map.get(id))
            .copied()
            .collect();
        if from.len() > 1 {
            report.merged += 1;
            examples.push(Example {
                kind: Kind::Merged,
                label: after_labels.get(core).cloned().unwrap_or_default(),
                members: members.len(),
                into: from.len(),
            });
        }
    }

    // Biggest changes first; a diff with one interesting group should lead with it.
    examples.sort_by(|a, b| b.members.cmp(&a.members).then(a.label.cmp(&b.label)));
    examples.truncate(10);
    report.examples = examples;

    Ok(report)
}

/// `core → sorted members`.
fn groups(map: &HashMap<i32, i32>) -> BTreeMap<i32, Vec<i32>> {
    let mut groups: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for (observation, core) in map {
        groups.entry(*core).or_default().push(*observation);
    }
    for members in groups.values_mut() {
        members.sort_unstable();
    }
    groups
}

/// `observation → sorted group-mates`, for comparing what an observation's group looks like.
fn group_mates(groups: &BTreeMap<i32, Vec<i32>>) -> HashMap<i32, Vec<i32>> {
    let mut mates = HashMap::new();
    for members in groups.values() {
        for observation in members {
            mates.insert(*observation, members.clone());
        }
    }
    mates
}

async fn membership(db: &database::DbHandle, table: &str) -> anyhow::Result<HashMap<i32, i32>> {
    let rows = db
        .conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT observation_id, core_id FROM \"{table}\""),
        ))
        .await
        .map_err(|error| anyhow::anyhow!("cannot read snapshot table {table}: {error}"))?;

    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        map.insert(row.try_get_by_index::<i32>(0)?, row.try_get_by_index::<i32>(1)?);
    }
    Ok(map)
}

async fn labels(db: &database::DbHandle, table: &str) -> anyhow::Result<HashMap<i32, String>> {
    let rows = db
        .conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT id, label FROM \"{table}\""),
        ))
        .await
        .map_err(|error| anyhow::anyhow!("cannot read snapshot table {table}: {error}"))?;

    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        map.insert(
            row.try_get_by_index::<i32>(0)?,
            row.try_get_by_index::<String>(1)?,
        );
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn table(db: &database::DbHandle, name: &str, columns: &str, rows: &str) {
        db.conn
            .execute_unprepared(&format!("CREATE TABLE \"{name}\" ({columns})"))
            .await
            .expect("table should be creatable");
        if !rows.is_empty() {
            db.conn
                .execute_unprepared(&format!("INSERT INTO \"{name}\" VALUES {rows}"))
                .await
                .expect("rows should insert");
        }
    }

    #[tokio::test]
    async fn reports_a_split_and_a_merge() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let before = Snapshot {
            suffix: "cmp_before".to_string(),
        };
        let after = Snapshot {
            suffix: "cmp_after".to_string(),
        };

        // Before: {1,2} · {3} · {4} · {5}
        table(&db, &before.membership(), "observation_id int, core_id int",
              "(1,10),(2,10),(3,20),(4,30),(5,40)").await;
        table(&db, &before.cores(), "id int, label text",
              r#"(10,'Train Dreams'),(20,'Dune'),(30,'Dune'),(40,'Wojak')"#).await;
        // After: {1} · {2} · {3,4} · {5} — the first group split, the middle two merged.
        table(&db, &after.membership(), "observation_id int, core_id int",
              "(1,11),(2,12),(3,21),(4,21),(5,40)").await;
        table(&db, &after.cores(), "id int, label text",
              r#"(11,'Train Dreams'),(12,'Train Dreams'),(21,'Dune'),(40,'Wojak')"#).await;

        let report = compare(&db, &before, &after).await.expect("compare should succeed");

        assert_eq!(report.observations, 5, "{report:?}");
        assert_eq!(report.split, 1, "{report:?}");
        assert_eq!(report.merged, 1, "{report:?}");
        assert_eq!(report.unchanged, 1, "the {{5}} group is identical: {report:?}");
        assert_eq!(report.moved, 4, "{report:?}");
        assert_eq!(report.cores_before, 4);
        assert_eq!(report.cores_after, 4);
        assert!(!report.identical());
        assert_eq!(report.examples.len(), 2, "{report:?}");
    }

    #[tokio::test]
    async fn identical_runs_report_no_differences() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let before = Snapshot {
            suffix: "same_before".to_string(),
        };
        let after = Snapshot {
            suffix: "same_after".to_string(),
        };

        table(&db, &before.membership(), "observation_id int, core_id int",
              "(1,10),(2,10),(3,20)").await;
        table(&db, &before.cores(), "id int, label text", r#"(10,'A'),(20,'B')"#).await;
        // Different core ids, same grouping — which is what a rebuild of the same corpus produces.
        table(&db, &after.membership(), "observation_id int, core_id int",
              "(1,71),(2,71),(3,82)").await;
        table(&db, &after.cores(), "id int, label text", r#"(71,'A'),(82,'B')"#).await;

        let report = compare(&db, &before, &after).await.expect("compare should succeed");

        assert!(report.identical(), "{report:?}");
        assert_eq!(report.unchanged, 2, "{report:?}");
        assert!(report.examples.is_empty(), "{report:?}");
    }

    #[tokio::test]
    async fn a_missing_snapshot_names_the_table() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let missing = Snapshot {
            suffix: "cmp_missing".to_string(),
        };

        let error = compare(&db, &missing, &missing)
            .await
            .expect_err("a missing snapshot should fail");

        assert!(
            error.to_string().contains("core_knowledge_membership__cmp_missing"),
            "{error}"
        );
    }
}
