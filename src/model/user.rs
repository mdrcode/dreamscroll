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
}

impl ActiveModelBehavior for ActiveModel {}
