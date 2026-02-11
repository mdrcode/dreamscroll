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

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub summary: String,
    pub details: String,

    #[sea_orm(has_one)]
    pub illumination_meta: HasOne<illumination_meta::Entity>,

    #[sea_orm(has_many)]
    pub xqueries: HasMany<xquery::Entity>,

    #[sea_orm(has_many)]
    pub knodes: HasMany<knode::Entity>,

    #[sea_orm(has_many)]
    pub social_medias: HasMany<social_media::Entity>,

    #[sea_orm(has_one)]
    pub search_index: HasOne<search_index::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
