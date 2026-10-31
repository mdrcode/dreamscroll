use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// One observation: what a single capture said. Owned by a user and never rewritten — the only
/// column a merge touches is `core_id`.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user_knowledge_entities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,
    #[sea_orm(unique_key = "user_knowledge_entity_position")]
    pub illumination_id: i32,
    #[sea_orm(unique_key = "user_knowledge_entity_position")]
    pub position: i32,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    pub name: String,
    pub entity_type: String,
    #[sea_orm(nullable)]
    pub description: Option<String>,
    #[sea_orm(nullable)]
    pub search_query: Option<String>,
    #[sea_orm(nullable)]
    pub platform: Option<String>,
    #[sea_orm(nullable)]
    pub platform_handle: Option<String>,
    #[sea_orm(nullable)]
    pub platform_display_name: Option<String>,

    /// The referent this observation was linked to. Null until `build_core` runs.
    #[sea_orm(nullable)]
    pub core_id: Option<i32>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    #[sea_orm(belongs_to, from = "illumination_id", to = "id")]
    pub illumination: HasOne<illumination::Entity>,

    #[sea_orm(belongs_to, from = "core_id", to = "id")]
    pub core: HasOne<core_knowledge_entity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
