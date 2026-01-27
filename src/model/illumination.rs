use sea_orm::entity::prelude::*;

use super::capture;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "illuminations6")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub capture_id: i32,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    pub provider_name: String,

    pub summary: String,
    pub details: String,

    #[sea_orm(has_many)]
    pub k_nodes: HasMany<super::k_node::Entity>,

    #[sea_orm(has_many)]
    pub x_queries: HasMany<super::x_query::Entity>,

    pub raw_content: String,
}

impl ActiveModelBehavior for ActiveModel {}
