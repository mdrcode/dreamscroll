use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "spark_links")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub spark_cluster_id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "spark_cluster_id", to = "id")]
    pub spark_cluster: HasOne<spark_cluster::Entity>,

    pub url: String,
    pub commentary: String,
}

impl ActiveModelBehavior for ActiveModel {}
