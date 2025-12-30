use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "illumination")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub capture_id: i32,
    pub provider: String,
    pub content: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::capture::Entity",
        from = "Column::CaptureId",
        to = "super::capture::Column::Id"
    )]
    Capture,
}

impl Related<super::capture::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Capture.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
