use sea_orm::entity::prelude::*;
use serde::Serialize;

use super::capture;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
#[sea_orm(table_name = "media")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub capture_id: Option<i32>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub filename: String,
}

impl ActiveModelBehavior for ActiveModel {}
