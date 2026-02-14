use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    #[sea_orm(unique)]
    pub username: String,

    #[sea_orm(nullable, column_type = "Text", default_value = "REPLACE_ME")]
    pub email: String,

    pub password_hash: String, // would secrecy::Secret<String> be better?

    #[sea_orm(column_type = "Boolean", default_value = "false")]
    pub is_admin: bool,

    /// Opaque 8-character base36 prefix for this user's uploads in storage.
    /// Generated once at account creation. Used to scope storage access.
    #[sea_orm(unique)]
    pub storage_shard: String,
}

impl ActiveModelBehavior for ActiveModel {}

/// Generates a random base36 string (a-z, 0-9) of num_chars length for use as
/// a user's storage shard prefix. Uses UUID v4 bytes as the entropy source.
/// Collisions will happen when number of users is low millions. If too many
/// collisions, just increase num_chars.
pub fn generate_storage_shard(num_chars: usize) -> String {
    if num_chars > 12 {
        panic!("generate_storage_shard: num_chars too large for current entropy source, max is 12");
    }
    let bytes = uuid::Uuid::new_v4();
    let mut value = u64::from_le_bytes(bytes.as_bytes()[..8].try_into().unwrap());
    let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut result = String::with_capacity(num_chars);
    for _ in 0..num_chars {
        result.push(charset[(value % 36) as usize] as char);
        value /= 36;
    }
    result
}
