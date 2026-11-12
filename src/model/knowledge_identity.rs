use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One authority's annotation of one entity with one durable identity.
///
/// An identity is the pair `(scheme, external_id)` — a Wikidata item, a platform account, a
/// Wikimedia file. The row says **this entity is named that**, and carries the authority's answer
/// about it: label, description, classes, method and confidence.
///
/// Rows are per provider, not per key. Two authorities may name the same key for the same entity;
/// that corroboration has its own confidence and method, and a resolver's full replace must not be
/// able to delete another provider's annotation. A key may also be held by several entities — one
/// Wikidata item can name an album and cover its songs — which is what `proximity` asks about.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_identities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The entity this annotation is about. An identity is never free-floating.
    pub knowledge_entity_id: i32,

    /// The namespace: `wikidata`, `wikimedia`, `x_twitter`.
    pub scheme: String,
    /// The identifier in its comparable form — `Q104919`, `bandy1803`.
    pub external_id: String,

    #[sea_orm(nullable)]
    pub label: Option<String>,
    #[sea_orm(nullable)]
    pub description: Option<String>,
    pub classes: Vec<String>,

    /// Who supplied the annotation: `illuminator` when an observation showed the handle, a
    /// resolver's id when one answered, `model` when a guess produced it.
    pub provider: String,
    /// How the annotation was reached: `observation`, a Wikidata property (`P2002`, `P2003`, …) when
    /// the answer came from a handle join, or the ladder step (`exact_label`, `wikipedia_title`,
    /// `search_lookup`, `search_ranked`, `search_hint`, `ranked`).
    #[sea_orm(nullable)]
    pub method: Option<String>,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,

    /// The observation that carried it, when a capture showed the handle. `None` for an authority
    /// answer.
    #[sea_orm(nullable)]
    pub user_observed_entity_id: Option<i32>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub observed_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "knowledge_entity_id", to = "id")]
    pub knowledge_entity: HasOne<crate::model::knowledge_entity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
