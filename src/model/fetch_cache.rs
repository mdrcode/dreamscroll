use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// What became of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Outcome {
    /// The provider answered **with a payload**. Not a decision: whether it holds what the caller
    /// wanted is the caller's decision, and it may reject the lot.
    #[sea_orm(string_value = "found")]
    Found,
    /// The provider answered with nothing — a cached negative.
    #[sea_orm(string_value = "empty")]
    Empty,
    /// The provider did not answer.
    #[sea_orm(string_value = "failed")]
    Failed,
    /// The provider answered, but not in a way worth trusting — a search that never reached its
    /// backend, a body carrying an error alongside a 200. The payload is still handed over, because
    /// it may hold something useful, but the answer expires quickly instead of standing for a week.
    #[sea_orm(string_value = "degraded")]
    Degraded,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::Empty => "empty",
            Self::Failed => "failed",
            Self::Degraded => "degraded",
        }
    }

    /// The stored form, back, for readers that walk a raw row rather than a model.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "found" => Some(Self::Found),
            "empty" => Some(Self::Empty),
            "failed" => Some(Self::Failed),
            "degraded" => Some(Self::Degraded),
            _ => None,
        }
    }

    /// Whether the answer is worth parsing. A degraded answer still carries a payload.
    pub fn answered(self) -> bool {
        matches!(self, Self::Found | Self::Degraded)
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

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
    pub outcome: Outcome,
    /// The raw provider response for this query, kept so parsing can be re-run without refetching.
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub payload: Option<Json>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub fetched_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
