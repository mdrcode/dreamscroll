use sea_orm::entity::prelude::*;

use super::capture;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "search_index")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub capture_id: i32,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub raw_capture_content: String,
}

impl ActiveModelBehavior for ActiveModel {}
