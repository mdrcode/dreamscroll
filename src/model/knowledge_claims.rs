use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A claim: this entity is what that identity names.
///
/// The edge of the many-to-many, not the identity itself — the identity and the authority's answer
/// about it live in `knowledge_identities`, and this row records that *this* entity asserts it and
/// whose observation carried it.
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

    /// Who supplied this edge: a resolver's id, `illuminator` for an observation, `model` for a guess.
    ///
    /// Per edge, unlike the identity's `provider`, so two authorities may assert one identity and
    /// both rows survive.
    pub provider: String,
    /// How this edge was reached: the ladder step, or `observation` for a platform link.
    #[sea_orm(nullable)]
    pub method: Option<String>,

    /// Provenance when the claim came from an observation.
    #[sea_orm(nullable)]
    pub user_observed_entity_id: Option<i32>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub observed_at: DateTime<Utc>,

    #[sea_orm(nullable)]
    pub retracted_at: Option<DateTime<Utc>>,

    #[sea_orm(belongs_to, from = "knowledge_entity_id", to = "id")]
    pub knowledge_entity: HasOne<knowledge_entity::Entity>,
    #[sea_orm(belongs_to, from = "knowledge_identity_id", to = "id")]
    pub identity: HasOne<knowledge_identity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
