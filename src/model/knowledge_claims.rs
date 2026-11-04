use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A claim: this entity is what that identity names.
///
/// The edge of the many-to-many, not the identity itself — the identity and the authority's answer
/// about it live in `knowledge_identities`, and this row records that *this* entity asserts it, with
/// the surface as a capture wrote it and who captured it.
///
/// Append-only, with one exception: `retracted_at` records that the claim stopped being believed.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_claims")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    pub knowledge_entity_id: i32,
    pub knowledge_identity_id: i32,

    /// The identifier as written, when a capture carried it verbatim (`@bandy1803`).
    #[sea_orm(nullable)]
    pub surface: Option<String>,

    /// What the question was about: `handle` or `text`. Corroboration counts one of each, so
    /// independence is a property of the evidence rather than of which provider answered.
    pub basis: String,

    /// Provenance when the claim came off a capture: which observation, whose capture, when.
    #[sea_orm(nullable)]
    pub user_observed_entity_id: Option<i32>,
    #[sea_orm(nullable)]
    pub source_user_id: Option<i32>,
    #[sea_orm(nullable)]
    pub retracted_at: Option<DateTime<Utc>>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub observed_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "knowledge_entity_id", to = "id")]
    pub knowledge_entity: HasOne<knowledge_entity::Entity>,
    #[sea_orm(belongs_to, from = "knowledge_identity_id", to = "id")]
    pub identity: HasOne<knowledge_identity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
