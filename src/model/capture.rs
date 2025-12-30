use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "captures")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::media::Entity")]
    Media,
    #[sea_orm(has_many = "super::illumination::Entity")]
    Illumination,
}

impl Related<super::media::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Media.def()
    }
}

impl Related<super::illumination::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Illumination.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
