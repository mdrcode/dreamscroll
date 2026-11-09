use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One answered request, per query, for one provider.
///
/// This table is the reason we can iterate on parsing and ranking for free, and the reason we
/// cannot spam a third party by accident: the executor checks it before every transport call, so a
/// warm cache means zero requests.
///
/// `shape` is part of the key on purpose. It hashes *how* the request was built (endpoint, language,
/// limit, property list). Change the construction and the key changes, so a stale "found nothing"
/// cannot outlive the query that produced it.
///
/// `outcome = 'empty'` is a cached answer, not a missing row: the long tail is a third of the
/// corpus, and re-asking a question we know the answer to is exactly the spam this table prevents.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "fetch_cache")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "fetch_cache_key")]
    pub provider: String,
    #[sea_orm(unique_key = "fetch_cache_key")]
    pub kind: String,
    #[sea_orm(unique_key = "fetch_cache_key")]
    pub query: String,
    #[sea_orm(unique_key = "fetch_cache_key")]
    pub shape: String,
    /// found | empty | failed
    pub outcome: String,
    /// The raw provider response for this query, kept so parsing can be re-run without refetching.
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub payload: Option<Json>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub fetched_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
