use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// A thing the world names, independent of whether we have an entity for it.
///
/// `(scheme, external_id)` is the identity: a Wikidata item, a platform account, a Wikimedia file.
/// The authority's answer about it — label, description, classes, and how we came by it — belongs
/// here rather than on every claim that cites it, because one identity may be claimed by several
/// entities: an album's article is the identity of the album and the coverage of each of its songs.
///
/// Unique on the pair. That is the invariant this table actually owns; nothing here says an identity
/// belongs to one entity.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_identities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The namespace: `wikidata`, `wikimedia`, `x_twitter`.
    pub scheme: String,
    /// The identifier in its comparable form — `Q104919`, `bandy1803`.
    pub external_id: String,

    /// What the authority said about it. Null when the identity came off a capture, which knows only
    /// the handle.
    #[sea_orm(nullable)]
    pub label: Option<String>,
    #[sea_orm(nullable)]
    pub description: Option<String>,
    pub classes: Vec<String>,

    /// Who answered and how: `wikidata` | `wikimedia` | `platform`, plus `exact_label` | `p2002` |
    /// `capture`.
    pub source: String,
    #[sea_orm(nullable)]
    pub method: Option<String>,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub first_seen: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
