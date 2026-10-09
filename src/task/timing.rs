use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Set};
use strum::{AsRefStr, Display};

use super::{Task, TaskRunStatus};
use crate::{database, model};

/// Minimum successful samples required before exposing a processing estimate
/// to clients. The aggregate remains stored with fewer samples.
pub const MINIMUM_CLIENT_ESTIMATE_SAMPLES: i64 = 5;
const TIMING_SAMPLE_LIMIT: i64 = 30;

#[derive(Clone, Copy, Debug, Display, Eq, PartialEq, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Measure {
    QueueWait,
    ProcessingSuccessful,
}

/// Aggregate timing estimate used by task-status events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskTimingEstimate {
    pub(crate) task_type: String,
    pub(crate) measure: Measure,
    pub(crate) sample_count: i64,
    pub(crate) duration_ms_avg: i64,
    pub(crate) duration_ms_p50: i64,
    pub(crate) duration_ms_p75: i64,
    pub(crate) duration_ms_p90: i64,
}

impl TaskTimingEstimate {
    /// Return the processing estimate when enough history makes it useful to clients.
    pub fn client_processing_duration_ms(&self) -> Option<i64> {
        (self.sample_count >= MINIMUM_CLIENT_ESTIMATE_SAMPLES).then_some(self.duration_ms_p50)
    }
}

pub async fn get_timing_estimate(
    db: &database::DbHandle,
    task_type: &str,
    operation_type: Measure,
) -> Option<TaskTimingEstimate> {
    model::task_run_timing::Entity::find()
        .filter(model::task_run_timing::Column::TaskType.eq(task_type))
        .filter(model::task_run_timing::Column::OperationType.eq(operation_type.as_ref()))
        .one(&db.conn)
        .await
        .ok()
        .flatten()
        .map(|row| TaskTimingEstimate {
            task_type: row.task_type,
            measure: operation_type,
            sample_count: row.sample_count,
            duration_ms_avg: row.duration_ms_avg,
            duration_ms_p50: row.duration_ms_p50,
            duration_ms_p75: row.duration_ms_p75,
            duration_ms_p90: row.duration_ms_p90,
        })
}

/// Recalculate one update-in-place timing aggregate from the most recent 30
/// relevant task-run rows.
pub async fn refresh_timing_measures<T: Task>(
    db: &database::DbHandle,
    status: TaskRunStatus,
) -> anyhow::Result<()> {
    let Some((operation_type, filter, duration)) = (match status {
        TaskRunStatus::InProgress => Some((
            Measure::QueueWait,
            "processing_started_at IS NOT NULL",
            "EXTRACT(EPOCH FROM (processing_started_at - created_at)) * 1000",
        )),
        TaskRunStatus::CompleteSuccess => Some((
            Measure::ProcessingSuccessful,
            "status_code = 4 AND success_duration_ms IS NOT NULL",
            "success_duration_ms",
        )),
        _ => None,
    }) else {
        return Ok(());
    };
    let sql = format!(
        "WITH recent AS (SELECT {duration} AS duration_ms FROM task_runs WHERE task_type = $1 AND {filter} ORDER BY updated_at DESC LIMIT {TIMING_SAMPLE_LIMIT}) SELECT COUNT(*)::BIGINT AS sample_count, COALESCE(ROUND(AVG(duration_ms)), 0)::BIGINT AS duration_ms_avg, COALESCE(PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY duration_ms), 0)::BIGINT AS duration_ms_p50, COALESCE(PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY duration_ms), 0)::BIGINT AS duration_ms_p75, COALESCE(PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY duration_ms), 0)::BIGINT AS duration_ms_p90 FROM recent"
    );
    let statement = sea_orm::Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
        vec![sea_orm::Value::String(Some(T::task_type().to_owned()))],
    );
    let result = db.conn.query_one_raw(statement).await?.unwrap();
    let sample_count: i64 = result.try_get_by("sample_count")?;
    if sample_count == 0 {
        return Ok(());
    }
    let active = model::task_run_timing::ActiveModel {
        task_type: Set(T::task_type().to_owned()),
        operation_type: Set(operation_type.as_ref().to_owned()),
        sample_count: Set(sample_count),
        duration_ms_avg: Set(result.try_get_by("duration_ms_avg")?),
        duration_ms_p50: Set(result.try_get_by("duration_ms_p50")?),
        duration_ms_p75: Set(result.try_get_by("duration_ms_p75")?),
        duration_ms_p90: Set(result.try_get_by("duration_ms_p90")?),
        ..Default::default()
    };
    model::task_run_timing::Entity::insert(active)
        .on_conflict(
            sea_orm::sea_query::OnConflict::columns([
                model::task_run_timing::Column::TaskType,
                model::task_run_timing::Column::OperationType,
            ])
            .update_columns([
                model::task_run_timing::Column::SampleCount,
                model::task_run_timing::Column::DurationMsAvg,
                model::task_run_timing::Column::DurationMsP50,
                model::task_run_timing::Column::DurationMsP75,
                model::task_run_timing::Column::DurationMsP90,
                model::task_run_timing::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec(&db.conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_measure_names_are_stable() {
        assert_eq!(Measure::QueueWait.as_ref(), "queue_wait");
        assert_eq!(
            Measure::ProcessingSuccessful.as_ref(),
            "processing_successful"
        );
        assert_eq!(Measure::QueueWait.to_string(), "queue_wait");
    }
}
