use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "captures")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<super::user::Entity>,

    #[sea_orm(has_many)]
    pub medias: HasMany<super::media::Entity>,

    #[sea_orm(has_many)]
    pub illuminations: HasMany<super::illumination::Entity>,

    #[sea_orm(has_many)]
    pub xqueries: HasMany<super::xquery::Entity>,

    #[sea_orm(has_many)]
    pub knodes: HasMany<super::knode::Entity>,

    #[sea_orm(has_many)]
    pub social_medias: HasMany<super::social_media::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
