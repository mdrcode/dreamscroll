use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::illumination;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "k_nodes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub illumination_id: i32,

    #[sea_orm(belongs_to, from = "illumination_id", to = "id")]
    pub illumination: HasOne<illumination::Entity>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    pub name: String,
    pub description: String,
    #[sea_orm(column_name = "type")]
    pub k_type: String,
}

impl ActiveModelBehavior for ActiveModel {}
