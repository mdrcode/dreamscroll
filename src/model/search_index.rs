use sea_orm::entity::prelude::*;

use super::{capture, user};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "search_index")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub content: String,
}

impl ActiveModelBehavior for ActiveModel {}
