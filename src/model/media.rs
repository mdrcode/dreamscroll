use sea_orm::entity::prelude::*;
use uuid::Uuid;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "medias")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,

    pub bytes: i64,
    pub mime_type: Option<String>,
    pub hash_blake3: Option<String>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    // Path is conceptually /[storage_bucket/]storage_user_shard/storage_uuid
    // but different providers may modify that mapping. UrlMaker converts
    // these fields to an actual URL for the browser or API.
    pub storage_provider: String,
    pub storage_bucket: Option<String>,
    pub storage_user_shard: String,
    pub storage_uuid: Uuid,
    pub storage_extension: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
