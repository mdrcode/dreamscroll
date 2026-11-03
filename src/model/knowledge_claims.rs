use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A claim: an answer about what this entity is, and who gave it.
///
/// The one kind of evidence that is a fact about the world rather than a judgement about our own
/// rows. The identity invariant lives here, where the claims are: no two entities may hold a live
/// claim to the same `(scheme, external_id)`.
///
/// Append-only, with one exception: `retracted_at` records that a claim stopped being believed, and
/// it is what the unique index is partial on. Retraction is a fact with a timestamp, not a status.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_claims")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub knowledge_entity_id: i32,

    /// The namespace the identifier lives in: `wikidata`, `wikimedia`, `x_twitter`.
    pub scheme: String,
    /// The identifier in its comparable form — `Q104919`, `bandy1803`. Comparable is the point: the
    /// unique index is what says one account has one handle.
    pub external_id: String,
    /// The identifier as written, when a capture carried it verbatim (`@bandy1803`).
    #[sea_orm(nullable)]
    pub surface: Option<String>,

    /// Who asserted it: `wikidata`, `wikimedia`, `platform`.
    pub source: String,
    /// How we got it: `exact_label`, `p2002`, `capture`.
    #[sea_orm(nullable)]
    pub method: Option<String>,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,

    /// Provenance when the claim came off a capture.
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
}

impl ActiveModelBehavior for ActiveModel {}
