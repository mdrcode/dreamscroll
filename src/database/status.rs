//! A read-only report of the schema and the graph.
//!
//! This is the command that answers "what is production actually like?" without a proxy, a
//! dumped secret, or ad-hoc SQL. It runs in-region as easily as locally because it needs only
//! `PostgresConfig`: the same credentials the schema job already has.

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use serde::Serialize;

use super::REQUIRED_SCHEMA_VERSION;

/// Tables an old shape left behind. Each is retired by a guarded step in `sync_schema`, so their
/// presence is the cheapest signal of a pending migration (and of clutter after a reset).
const LEGACY_TABLES: &[(&str, &str)] = &[
    (
        "knowledge_entities_v1",
        "43: adopt the v1 entity table as observations",
    ),
    (
        "knowledge_proximities",
        "41: drop persisted proximity candidates",
    ),
    ("knowledge_claims", "39: retire the claim edge"),
    ("knowledge_merges", "28: retire the merge audit"),
    (
        "resolution_cache",
        "31: rename resolution_cache to fetch_cache",
    ),
    (
        "knowledge_index",
        "32: rename knowledge_index to knowledge_blocking_index",
    ),
    (
        "knowledge_user",
        "16: move staging observations into user_observed_entities",
    ),
    ("xqueries", "33: drop the legacy tables"),
    ("illuminations_legacy", "33: drop the legacy tables"),
];

/// Tables worth counting on every check. Missing ones are skipped, so this works on a fresh
/// database as well as a current one.
const COUNTED_TABLES: &[&str] = &[
    "captures",
    "illuminations",
    "user_observed_entities",
    "knowledge_entities",
    "knowledge_identities",
    "knowledge_verdicts",
];

#[derive(Debug, Serialize)]
pub struct Status {
    pub applied_schema_version: Option<i32>,
    pub required_schema_version: i32,
    /// Guarded steps that would fire on the next sync, most important first.
    pub pending: Vec<String>,
    pub legacy_tables: Vec<String>,
    /// The v1-only column that makes schema 43 fire.
    pub v1_entity_table: bool,
    pub row_counts: BTreeMap<String, i64>,
    pub observations_linked: Option<i64>,
    pub entities_by_status: BTreeMap<String, i64>,
    pub identities_by_provider: BTreeMap<String, i64>,
}

impl Status {
    pub fn in_sync(&self) -> bool {
        self.applied_schema_version == Some(self.required_schema_version)
    }
}

pub async fn status(conn: &DatabaseConnection) -> anyhow::Result<Status> {
    let applied_schema_version = applied_version(conn).await?;

    let mut legacy_tables = Vec::new();
    let mut pending = Vec::new();
    for (table, step) in LEGACY_TABLES {
        if table_exists(conn, table).await? {
            legacy_tables.push((*table).to_owned());
            push_unique(&mut pending, step);
        }
    }

    if applied_schema_version.is_none() {
        pending.insert(
            0,
            "apply the schema: the version marker is missing".to_owned(),
        );
    } else if !applied_schema_version.is_some_and(|applied| applied >= REQUIRED_SCHEMA_VERSION) {
        pending.insert(
            0,
            format!(
                "sync required: applied {}, required {REQUIRED_SCHEMA_VERSION}",
                applied_schema_version.unwrap_or_default()
            ),
        );
    }

    let v1_entity_table = column_exists(conn, "knowledge_entities", "capture_id").await?;
    if v1_entity_table {
        push_unique(
            &mut pending,
            "43: adopt the v1 entity table as observations",
        );
    }
    if table_exists(conn, "knowledge_identities").await?
        && !column_exists(conn, "knowledge_identities", "knowledge_entity_id").await?
    {
        push_unique(&mut pending, "39: retire the claim edge");
    }

    let mut row_counts = BTreeMap::new();
    for table in COUNTED_TABLES {
        if table_exists(conn, table).await? {
            row_counts.insert((*table).to_owned(), count(conn, table).await?);
        }
    }

    let observations_linked = if table_exists(conn, "user_observed_entities").await? {
        Some(
            count_where(
                conn,
                "user_observed_entities",
                "knowledge_entity_id IS NOT NULL",
            )
            .await?,
        )
    } else {
        None
    };

    let entities_by_status = if row_counts.contains_key("knowledge_entities") {
        grouped_counts(conn, "knowledge_entities", "status").await?
    } else {
        BTreeMap::new()
    };
    let identities_by_provider = if row_counts.contains_key("knowledge_identities") {
        grouped_counts(conn, "knowledge_identities", "provider").await?
    } else {
        BTreeMap::new()
    };

    Ok(Status {
        applied_schema_version,
        required_schema_version: REQUIRED_SCHEMA_VERSION,
        pending,
        legacy_tables,
        v1_entity_table,
        row_counts,
        observations_linked,
        entities_by_status,
        identities_by_provider,
    })
}

fn push_unique(pending: &mut Vec<String>, step: &str) {
    if !pending.iter().any(|existing| existing == step) {
        pending.push(step.to_owned());
    }
}

async fn applied_version(conn: &DatabaseConnection) -> anyhow::Result<Option<i32>> {
    if !table_exists(conn, "dreamscroll_schema_version").await? {
        return Ok(None);
    }
    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT version FROM dreamscroll_schema_version WHERE id = 1",
        ))
        .await?;
    Ok(match row {
        Some(row) => Some(row.try_get_by_index(0)?),
        None => None,
    })
}

async fn table_exists(conn: &DatabaseConnection, table: &str) -> anyhow::Result<bool> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT to_regclass($1)::text",
            [table.into()],
        ))
        .await?
        .expect("to_regclass always returns one row");
    let found: Option<String> = row.try_get_by_index(0)?;
    Ok(found.is_some())
}

async fn column_exists(
    conn: &DatabaseConnection,
    table: &str,
    column: &str,
) -> anyhow::Result<bool> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 FROM information_schema.columns \
              WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2 \
              LIMIT 1",
            [table.into(), column.into()],
        ))
        .await?;
    Ok(row.is_some())
}

async fn count(conn: &DatabaseConnection, table: &str) -> anyhow::Result<i64> {
    count_where(conn, table, "TRUE").await
}

async fn count_where(
    conn: &DatabaseConnection,
    table: &str,
    predicate: &str,
) -> anyhow::Result<i64> {
    // `table` and `predicate` are crate constants, never caller input.
    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT count(*) FROM {table} WHERE {predicate}"),
        ))
        .await?
        .expect("count(*) always returns one row");
    Ok(row.try_get_by_index(0)?)
}

async fn grouped_counts(
    conn: &DatabaseConnection,
    table: &str,
    column: &str,
) -> anyhow::Result<BTreeMap<String, i64>> {
    // Constant identifiers only, for the same reason as `count_where`.
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!("SELECT {column}, count(*) FROM {table} GROUP BY 1 ORDER BY 1"),
        ))
        .await?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let key: String = row.try_get_by_index(0)?;
        let value: i64 = row.try_get_by_index(1)?;
        counts.insert(key, value);
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use crate::test_support::db_or_skip;

    #[tokio::test]
    async fn status_reports_the_current_schema_and_counts() {
        db_or_skip!(db);
        let status = super::status(&db.conn)
            .await
            .expect("status should succeed");

        assert!(
            status.in_sync(),
            "applied: {:?}",
            status.applied_schema_version
        );
        assert!(!status.v1_entity_table);
        assert!(
            status.row_counts.contains_key("captures"),
            "counts: {:?}",
            status.row_counts
        );
        assert!(status.observations_linked.is_some());
    }
}
