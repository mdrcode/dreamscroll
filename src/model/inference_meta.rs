use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "inference_metas")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    /// TaskRun.run_id, not its numeric database primary key.
    #[sea_orm(nullable)]
    pub task_run_id: Option<String>,
    #[sea_orm(nullable)]
    pub attempt: Option<i32>,
    #[sea_orm(nullable)]
    pub task_type: Option<String>,
    pub provider: String,
    pub backend: String,
    pub model: String,
    pub duration_ms: i64,
    #[sea_orm(nullable)]
    pub tokens_in: Option<i64>,
    #[sea_orm(nullable)]
    pub tokens_in_cached: Option<i64>,
    #[sea_orm(nullable)]
    pub tokens_out: Option<i64>,
    #[sea_orm(column_type = "JsonBinary", default_value = "{}")]
    pub params: Json,
}

impl ActiveModelBehavior for ActiveModel {}
