use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "illumination_raws")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,
    pub media_id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    #[sea_orm(belongs_to, from = "media_id", to = "id")]
    pub media: HasOne<media::Entity>,

    pub inference_id: String,
    pub prompt_version: String,
    pub provider_name: String,
    pub backend_name: String,
    pub model_id: String,
    pub duration_ms: i64,

    #[sea_orm(nullable)]
    pub provider_request_id: Option<String>,

    #[sea_orm(nullable, column_type = "JsonBinary")]
    pub provider_usage_json: Option<Json>,

    #[sea_orm(column_type = "JsonBinary")]
    pub content: Json,
}

impl ActiveModelBehavior for ActiveModel {}
