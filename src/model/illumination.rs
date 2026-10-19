use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "illuminations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,
    /// TaskRun that produced this projection; retries replace the same row.
    #[sea_orm(unique)]
    pub task_run_id: String,

    #[sea_orm(nullable)]
    pub inference_meta_id: Option<i32>,

    #[sea_orm(belongs_to, from = "inference_meta_id", to = "id")]
    pub inference_meta: HasOne<inference_meta::Entity>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub summary: String,
    pub details: String,
    pub suggested_searches: Option<Vec<String>>,

    #[sea_orm(has_many)]
    pub knowledge_entities: HasMany<knowledge_entity::Entity>,

    #[sea_orm(has_one)]
    pub search_index: HasOne<search_index::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
