use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "spark_metas")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub spark_id: i32,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "spark_id", to = "id")]
    pub spark: HasOne<spark::Entity>,

    pub provider_name: String,
    pub duration_ms: i64,
    pub input_capture_count: i32,

    #[sea_orm(nullable)]
    pub input_tokens: Option<i32>,

    #[sea_orm(nullable)]
    pub output_tokens: Option<i32>,

    #[sea_orm(nullable)]
    pub total_tokens: Option<i32>,

    #[sea_orm(nullable, column_type = "Text")]
    pub provider_usage_json: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
