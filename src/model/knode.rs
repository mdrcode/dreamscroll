use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "knodes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    #[sea_orm(belongs_to, from = "illumination_id", to = "id")]
    pub illumination: HasOne<illumination::Entity>,

    pub name: String,
    pub description: String,
    #[sea_orm(column_name = "type")] // "type" is a reserved keyword in Rust
    pub k_type: String,
}

impl ActiveModelBehavior for ActiveModel {}
