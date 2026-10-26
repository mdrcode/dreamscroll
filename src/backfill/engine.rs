use std::{future::Future, sync::Arc};

use anyhow::anyhow;
use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, Statement,
    sea_query::{Expr, ExprTrait, Order, Query},
};

use crate::{
    api::{self, ApiError},
    auth, database, model,
    task::{self, Task},
};

use super::{
    definition::{Backfill, BackfillTarget, BackfillTargets, TargetColumn},
    illuminate_v2_backfill::IlluminateV2Backfill,
    types::*,
};

const BATCH_SIZE: u64 = 100;

// Explicit registry dispatch preserves TaskMaster's concrete queue types.
enum RegisteredBackfill {
    IlluminateV2(IlluminateV2Backfill),
}

impl RegisteredBackfill {
    fn id(&self) -> &'static str {
        match self {
            Self::IlluminateV2(backfill) => backfill.id(),
        }
    }
}

fn definition(id: &str) -> Result<RegisteredBackfill, ApiError> {
    let backfill = IlluminateV2Backfill;
    if id == backfill.id() {
        Ok(RegisteredBackfill::IlluminateV2(backfill))
    } else {
        Err(ApiError::not_found(anyhow!("Unknown backfill {id:?}")))
    }
}

pub async fn pump(
    db: &database::DbHandle,
    task_master: &Arc<task::TaskMaster>,
    context: &auth::Context,
    req: BackfillPumpRequest,
) -> Result<BackfillPumpResponse, ApiError> {
    match definition(&req.backfill_id)? {
        RegisteredBackfill::IlluminateV2(backfill) => {
            pump_definition(
                db,
                &backfill,
                req.limit.unwrap_or(BATCH_SIZE),
                req.force,
                |task, id| task_master.submit_illuminate(context, task, Some(id)),
            )
            .await
        }
    }
}

pub(super) async fn select_targets<B: Backfill>(
    db: &database::DbHandle,
    backfill: &B,
    limit: u64,
) -> Result<Vec<BackfillTarget>, ApiError> {
    use model::task_run;

    let submitted = task_run::Entity::find()
        .select_only()
        .column(task_run::Column::Id)
        .filter(task_run::Column::BackfillId.eq(backfill.id()))
        .filter(task_run::Column::TaskType.eq(B::Task::task_type()))
        .filter(task_run::Column::EntityType.eq(B::Task::entity_type()))
        .filter(
            Expr::col((task_run::Entity, task_run::Column::EntityOwnerId))
                .eq(Expr::col((BackfillTargets, TargetColumn::OwnerId))),
        )
        .filter(
            Expr::col((task_run::Entity, task_run::Column::EntityId))
                .eq(Expr::col((BackfillTargets, TargetColumn::EntityId))),
        )
        .into_query();

    // Wrap the candidate relation so exclusion precedes the shared batch limit.
    // Do not skip live in-flight work: short-lived contention may fill a batch
    // and temporarily starve lower IDs. Accepted tradeoff; pump again later.
    let mut query = Query::select();
    query
        .columns([
            (BackfillTargets, TargetColumn::EntityId),
            (BackfillTargets, TargetColumn::OwnerId),
        ])
        .from_subquery(backfill.targets(), BackfillTargets)
        .and_where(Expr::exists(submitted).not())
        .order_by((BackfillTargets, TargetColumn::EntityId), Order::Desc)
        .order_by((BackfillTargets, TargetColumn::OwnerId), Order::Desc)
        .limit(limit);

    Ok(
        BackfillTarget::find_by_statement(db.conn.get_database_backend().build(&query))
            .all(&db.conn)
            .await?,
    )
}

pub(super) async fn pump_definition<B, F, Fut>(
    db: &database::DbHandle,
    backfill: &B,
    limit: u64,
    force: bool,
    submit: F,
) -> Result<BackfillPumpResponse, ApiError>
where
    B: Backfill,
    F: Fn(B::Task, &'static str) -> Fut,
    Fut: Future<Output = Result<task::SubmitOutcome, ApiError>>,
{
    if limit == 0 {
        return Err(ApiError::bad_request(anyhow!(
            "Candidate limit must be greater than zero"
        )));
    }
    ensure_not_dry(db, backfill.id(), force).await?;

    let targets = select_targets(db, backfill, limit).await?;
    let selected_count = targets.len();
    let mut enqueued_count = 0;
    let mut already_submitted_count = 0;
    let mut refused_in_flight_count = 0;
    let mut errors = Vec::new();

    for target in targets {
        match submit(backfill.make_task(target), backfill.id()).await {
            Ok(task::SubmitOutcome::Enqueued { .. }) => enqueued_count += 1,
            Ok(task::SubmitOutcome::AlreadySubmittedBackfill { .. }) => {
                already_submitted_count += 1
            }
            Ok(task::SubmitOutcome::RefusedAlreadyInFlight) => refused_in_flight_count += 1,
            Err(error) => {
                tracing::warn!(backfill_id = backfill.id(), entity_id = target.entity_id, error = ?error, "Backfill submission failed");
                errors.push(BackfillSubmissionError {
                    entity_id: target.entity_id,
                    error: error.into_inner().to_string(),
                });
            }
        }
    }

    let (pumped_at, dry_since) = record_pump(db, backfill.id(), selected_count == 0).await?;
    Ok(BackfillPumpResponse {
        backfill_id: backfill.id().to_owned(),
        selected_count,
        enqueued_count,
        already_submitted_count,
        refused_in_flight_count,
        errors,
        dry_since,
        pumped_at,
    })
}

/// Refuse the default pump once a dry streak is recorded. `force` is the
/// operator's acknowledgement that work may have appeared since; it only
/// bypasses this latch, never selection, deduplication, or in-flight refusal.
async fn ensure_not_dry(
    db: &database::DbHandle,
    backfill_id: &str,
    force: bool,
) -> Result<(), ApiError> {
    if force {
        return Ok(());
    }
    let Some(record) = model::backfill::Entity::find_by_id(backfill_id.to_owned())
        .one(&db.conn)
        .await?
    else {
        return Ok(());
    };
    let Some(dry_since) = record.dry_since else {
        return Ok(());
    };
    Err(ApiError::conflict(anyhow!(
        "Backfill {backfill_id:?} has had no eligible targets since {} (last pumped {}); \
         pass force=true (CLI --force) to pump again",
        format_time(dry_since),
        format_time(record.last_pumped_at),
    )))
}

/// Record this pump's observation in one atomic upsert.
///
/// A dry pump starts a streak if none is recorded and preserves the original
/// start otherwise; a non-dry pump clears it, so a forced pump that finds work
/// re-opens the backfill. Returns the recorded `(pumped_at, dry_since)`.
const RECORD_PUMP_SQL: &str = "\
INSERT INTO backfills (backfill_id, last_pumped_at, dry_since) \
VALUES ($1, now(), CASE WHEN $2 THEN now() END) \
ON CONFLICT (backfill_id) DO UPDATE SET \
    last_pumped_at = now(), \
    dry_since = CASE WHEN $2 THEN COALESCE(backfills.dry_since, now()) ELSE NULL END \
RETURNING last_pumped_at, dry_since";

async fn record_pump(
    db: &database::DbHandle,
    backfill_id: &str,
    dry: bool,
) -> Result<(DateTime<Utc>, Option<DateTime<Utc>>), ApiError> {
    let row = db
        .conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            RECORD_PUMP_SQL,
            [backfill_id.into(), dry.into()],
        ))
        .await?
        .ok_or_else(|| {
            ApiError::internal(anyhow!("Backfill pump record upsert returned no row"))
        })?;
    Ok((
        row.try_get_by("last_pumped_at")?,
        row.try_get_by("dry_since")?,
    ))
}

fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[derive(FromQueryResult)]
struct StatusCount {
    task_type: String,
    status_code: i32,
    count: i64,
}

pub async fn status(
    db: &database::DbHandle,
    backfill_id: &str,
) -> Result<BackfillStatusResponse, ApiError> {
    let definition = definition(backfill_id)?;
    let record = model::backfill::Entity::find_by_id(definition.id().to_owned())
        .one(&db.conn)
        .await?;
    let rows = model::task_run::Entity::find()
        .select_only()
        .column(model::task_run::Column::TaskType)
        .column(model::task_run::Column::StatusCode)
        .column_as(model::task_run::Column::Id.count(), "count")
        .filter(model::task_run::Column::BackfillId.eq(definition.id()))
        .group_by(model::task_run::Column::TaskType)
        .group_by(model::task_run::Column::StatusCode)
        .order_by_asc(model::task_run::Column::TaskType)
        .order_by_asc(model::task_run::Column::StatusCode)
        .into_model::<StatusCount>()
        .all(&db.conn)
        .await?;
    let counts = rows
        .into_iter()
        .map(|row| {
            Ok(BackfillStatusCount {
                task_type: row.task_type,
                status: task::TaskRunStatus::from_i32(row.status_code)
                    .map_err(ApiError::internal)?,
                count: row.count as u64,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(BackfillStatusResponse {
        backfill_id: definition.id().to_owned(),
        last_pumped_at: record.as_ref().map(|record| record.last_pumped_at),
        dry_since: record.and_then(|record| record.dry_since),
        counts,
    })
}

/// Terminally unsuccessful runs for a backfill, with the error each failing
/// layer reported.
///
/// Tagged runs are unique per logical task (`(backfill_id, logical_id)`), so each
/// row is the target's only tagged run: no later run can supersede a failure, and
/// there is nothing to collapse. Successes and in-flight work are excluded; see
/// `status` for those counts.
pub async fn failures(
    db: &database::DbHandle,
    backfill_id: &str,
) -> Result<BackfillFailuresResponse, ApiError> {
    let definition = definition(backfill_id)?;
    let rows = model::task_run::Entity::find()
        .filter(model::task_run::Column::BackfillId.eq(definition.id()))
        .filter(model::task_run::Column::StatusCode.is_in([
            task::TaskRunStatus::CompleteFailure.as_i32(),
            task::TaskRunStatus::SubmissionFailed.as_i32(),
        ]))
        .order_by_asc(model::task_run::Column::EntityId)
        .order_by_asc(model::task_run::Column::TaskType)
        .order_by_asc(model::task_run::Column::Id)
        .all(&db.conn)
        .await?;

    let failures = rows
        .into_iter()
        .map(|row| api::TaskRunInfo::try_from(row).map_err(ApiError::internal))
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(BackfillFailuresResponse {
        backfill_id: definition.id().to_owned(),
        failures,
    })
}
