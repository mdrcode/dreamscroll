use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "medias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: Option<i32>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    // Path suffix is conceptually [storage_bucket/][storage_shard/]storage_id
    // but different providers may modify that mapping.
    pub storage_provider: String,
    pub storage_bucket: Option<String>,
    pub storage_shard: Option<String>,
    pub storage_id: String,

    pub hash_blake3: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
