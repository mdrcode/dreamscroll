use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

// One row per illuminated occurrence, keyed by user. The table is `knowledge_user` (the per-user
// layer, see plan/knowledge-canonical-entities.md); the module keeps the noun `knowledge_entity`
// because the rows are entity mentions (`KnowledgeEntityInfo`, `/entity/{id}`), and the relation
// field reads `knowledge_entities`. The canonical entity is `knowledge_core`.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_user")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub capture_id: i32,
    #[sea_orm(unique_key = "knowledge_user_position")]
    pub illumination_id: i32,
    #[sea_orm(unique_key = "knowledge_user_position")]
    pub position: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "user_id", to = "id")]
    pub user: HasOne<user::Entity>,

    #[sea_orm(belongs_to, from = "capture_id", to = "id")]
    pub capture: HasOne<capture::Entity>,

    #[sea_orm(belongs_to, from = "illumination_id", to = "id")]
    pub illumination: HasOne<illumination::Entity>,

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
}

impl ActiveModelBehavior for ActiveModel {}
