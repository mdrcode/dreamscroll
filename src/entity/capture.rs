use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "capture")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub created_at: DateTime<Utc>,

    #[sea_orm(has_many)]
    pub medias: HasMany<super::media::Entity>,

    #[sea_orm(has_many)]
    pub illuminations: HasMany<super::illumination::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
