use sea_orm::entity::prelude::*;

use crate::storage;

use super::capture;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "medias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub capture_id: Option<i32>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    // Path is constructed as [storage_bucket/][storage_shard/]storage_id
    pub storage_bucket: Option<String>,
    pub storage_shard: Option<String>,
    pub storage_id: String,

    pub hash_blake3: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}

impl From<storage::StorageIdentity> for ActiveModelEx {
    fn from(storage_id: storage::StorageIdentity) -> Self {
        ActiveModel::builder()
            .set_storage_id(storage_id.provider_id)
            .set_storage_bucket(storage_id.provider_bucket)
            .set_storage_shard(storage_id.provider_shard)
    }
}
