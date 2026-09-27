use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// Update-in-place aggregate timing measures for one task type and operation.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_run_timing")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,

    #[sea_orm(unique_key = "task_run_timing_task_operation")]
    pub task_type: String,

    #[sea_orm(unique_key = "task_run_timing_task_operation")]
    pub operation_type: String,

    pub sample_count: i64,
    pub duration_ms_avg: i64,
    pub duration_ms_p50: i64,
    pub duration_ms_p75: i64,
    pub duration_ms_p90: i64,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
