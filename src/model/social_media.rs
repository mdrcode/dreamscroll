use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::capture;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "social_medias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub capture_id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub display_name: String,
    pub handle: String,
    pub platform: String,
}

impl ActiveModelBehavior for ActiveModel {}
