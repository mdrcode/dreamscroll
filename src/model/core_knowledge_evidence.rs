use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A fact that bears on a core, with where it came from.
///
/// Identity lives here rather than in typed columns on the core: a claim is evidence, and the
/// constraint that keeps two cores from claiming the same referent belongs where the claims are.
/// A partial unique index over the live payload (see `database::indexes::knowledge`) enforces it:
/// no two cores hold a live claim to the same `(scheme, id)`.
///
/// Append-only, with one exception: `retracted_at` records that a claim stopped being believed, and
/// it is what the unique index is partial on. Retraction is a fact with a timestamp, not a status.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "core_knowledge_evidence")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub core_id: i32,
    /// ontology | platform_link | description | collision | identity_claim
    /// | identity_rejection | merge
    pub kind: String,
    /// For identity kinds: `{"scheme": "wikidata", "id": "Q104919", …}`. The unique index reads
    /// these two keys, so the shape is a contract with `database::indexes::knowledge`.
    ///
    /// `jsonb`, not `json`: the payload is meant to be queried (`@>` containment, GIN), and `->>`
    /// alone would keep both types working until the first containment query silently did not.
    #[sea_orm(column_type = "JsonBinary")]
    pub value: Json,
    /// wikidata | wikimedia | platform | capture | model | user | manual
    pub source: String,
    #[sea_orm(nullable)]
    pub method: Option<String>,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,
    /// The other side of a pairwise fact: a collision, a merge's child.
    #[sea_orm(nullable)]
    pub related_core_id: Option<i32>,
    /// Provenance when the fact came from a capture.
    #[sea_orm(nullable)]
    pub user_entity_id: Option<i32>,
    #[sea_orm(nullable)]
    pub source_user_id: Option<i32>,
    #[sea_orm(nullable)]
    pub retracted_at: Option<DateTime<Utc>>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub observed_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "core_id", to = "id")]
    pub core: HasOne<core_knowledge_entity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
