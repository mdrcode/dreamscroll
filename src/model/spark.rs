use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sparks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(has_many)]
    pub spark_clusters: HasMany<spark_cluster::Entity>,

    #[sea_orm(has_many)]
    pub spark_output_refs: HasMany<spark_output_ref::Entity>,

    #[sea_orm(has_many)]
    pub spark_input_refs: HasMany<spark_input_ref::Entity>,

    #[sea_orm(has_one)]
    pub spark_meta: HasOne<spark_meta::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
