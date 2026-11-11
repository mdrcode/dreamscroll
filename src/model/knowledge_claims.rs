use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// What the question was about, for one claim: the identifier, or the text.
///
/// Corroboration counts independent *inputs*, and independence is this, not who answered. An
/// observation's handle and an authority asked about that handle are one chain however many schemes
/// they name; a name and a handle are two. Recorded when the claim is written, because that is the
/// only moment anything knows which question produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum ClaimBasis {
    /// A handle: an observation showed one, or the authority was asked about one.
    #[sea_orm(string_value = "handle")]
    Handle,
    /// A name, title or description.
    #[sea_orm(string_value = "text")]
    Text,
}

impl ClaimBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Handle => "handle",
            Self::Text => "text",
        }
    }

    /// The stored form, back, for readers that walk a raw row rather than a model.
    pub fn from_db(value: &str) -> Self {
        if value == Self::Handle.as_str() {
            Self::Handle
        } else {
            Self::Text
        }
    }
}

impl std::fmt::Display for ClaimBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

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

    /// What the question was about: `handle` or `text`. Corroboration counts one of each, so
    /// independence is a property of the evidence rather than of which provider answered.
    pub basis: ClaimBasis,

    /// Who supplied this edge: a resolver's id, `illuminator` for an observation, `model` for a guess.
    ///
    /// Per edge, unlike the identity's `provider`, so two authorities may assert one identity and
    /// both rows survive.
    pub provider: String,
    /// How this edge was reached: the ladder step, or `observation` for a platform link.
    #[sea_orm(nullable)]
    pub method: Option<String>,

    /// Provenance when the claim came from an observation: which observation, and when.
    #[sea_orm(nullable)]
    pub user_observed_entity_id: Option<i32>,
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
