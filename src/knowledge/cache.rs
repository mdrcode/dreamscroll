//! Cache by default: the only door to a third-party provider.
//!
//! Every request the resolver makes passes through here first. A warm cache means no network call at
//! all, and a *negative* answer is cached as deliberately as a positive one — the tail is a third of
//! the corpus, and re-asking a question we have already answered is the spam this exists to prevent.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::model::resolution_cache as cached;

/// Seven days for everything, which is short enough that a name gaining a Wikidata item is noticed
/// and long enough that a rerun is free.
pub const DEFAULT_TTL: Duration = Duration::days(7);

/// The one exception: a provider that could not be reached is not a fact about the world, and
/// caching "the provider was down" for a week would poison that part of the corpus.
pub const FAILED_TTL: Duration = Duration::minutes(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The provider answered **with candidates**. Not a claim: whether one of them is the referent
    /// is the ranking's decision, and it can reject them all.
    Found,
    /// The provider answered with nothing — a cached negative.
    Empty,
    /// The provider did not answer.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Found => "found",
            Outcome::Empty => "empty",
            Outcome::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "found" => Some(Outcome::Found),
            "empty" => Some(Outcome::Empty),
            "failed" => Some(Outcome::Failed),
            _ => None,
        }
    }

    fn ttl(self) -> Duration {
        match self {
            Outcome::Failed => FAILED_TTL,
            Outcome::Found | Outcome::Empty => DEFAULT_TTL,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub outcome: Outcome,
    /// The raw provider response, kept so parsing and ranking can be re-run without refetching.
    pub payload: Option<serde_json::Value>,
}

pub struct Cache<'a> {
    conn: &'a DatabaseConnection,
}

impl<'a> Cache<'a> {
    pub fn new(conn: &'a DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Answers for the queries that are still fresh. Anything absent is a miss, including rows that
    /// exist but have expired: a stale answer is not an answer.
    pub async fn get_many(
        &self,
        provider: &str,
        kind: &str,
        shape: &str,
        queries: &[String],
    ) -> anyhow::Result<HashMap<String, Answer>> {
        if queries.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = cached::Entity::find()
            .filter(cached::Column::Provider.eq(provider))
            .filter(cached::Column::Kind.eq(kind))
            .filter(cached::Column::Shape.eq(shape))
            .filter(cached::Column::Query.is_in(queries.iter().cloned()))
            .filter(cached::Column::ExpiresAt.gt(Utc::now()))
            .all(self.conn)
            .await?;

        let mut answers = HashMap::with_capacity(rows.len());
        for row in rows {
            let Some(outcome) = Outcome::parse(&row.outcome) else {
                continue;
            };
            answers.insert(row.query, Answer { outcome, payload: row.payload });
        }
        Ok(answers)
    }

    /// Record an answer. One row per logical request: an expired or re-fetched answer replaces the
    /// old one rather than accumulating history, because the payload is a cache, not evidence.
    pub async fn put(
        &self,
        provider: &str,
        kind: &str,
        shape: &str,
        query: &str,
        answer: &Answer,
    ) -> anyhow::Result<()> {
        let expires_at: DateTime<Utc> = Utc::now() + answer.outcome.ttl();
        let mut on_conflict = OnConflict::columns([
            cached::Column::Provider,
            cached::Column::Kind,
            cached::Column::Query,
            cached::Column::Shape,
        ]);
        on_conflict.update_columns([
            cached::Column::Outcome,
            cached::Column::Payload,
            cached::Column::FetchedAt,
            cached::Column::ExpiresAt,
        ]);

        cached::Entity::insert(cached::ActiveModel {
            provider: Set(provider.to_string()),
            kind: Set(kind.to_string()),
            query: Set(query.to_string()),
            shape: Set(shape.to_string()),
            outcome: Set(answer.outcome.as_str().to_string()),
            payload: Set(answer.payload.clone()),
            expires_at: Set(expires_at),
            ..Default::default()
        })
        .on_conflict(on_conflict)
        .exec_without_returning(self.conn)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;
    use serde_json::json;

    use super::*;

    fn answer(outcome: Outcome, payload: Option<serde_json::Value>) -> Answer {
        Answer { outcome, payload }
    }

    #[tokio::test]
    async fn stores_reads_and_expires() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let cache = Cache::new(&db.conn);
        let queries = vec!["Train Dreams".to_string(), "Wojak".to_string()];

        cache
            .put("wikidata", "label", "v1", "Train Dreams", &answer(Outcome::Found, Some(json!({"qid": "Q1"}))))
            .await
            .expect("put should succeed");
        cache
            .put("wikidata", "label", "v1", "Wojak", &answer(Outcome::Empty, None))
            .await
            .expect("put should succeed");

        let hits = cache.get_many("wikidata", "label", "v1", &queries).await.expect("get");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits["Train Dreams"].outcome, Outcome::Found);
        assert_eq!(hits["Wojak"].outcome, Outcome::Empty, "a negative is a cached answer");

        // Expire one row by hand; a stale answer must not be served.
        cached::Entity::update_many()
            .col_expr(cached::Column::ExpiresAt, sea_orm::sea_query::Expr::value(Utc::now() - Duration::seconds(1)))
            .filter(cached::Column::Query.eq("Wojak"))
            .exec(&db.conn)
            .await
            .expect("expiry update should succeed");

        let hits = cache.get_many("wikidata", "label", "v1", &queries).await.expect("get");
        assert_eq!(hits.len(), 1, "the expired row is a miss: {hits:?}");
        assert!(!hits.contains_key("Wojak"));
    }

    #[tokio::test]
    async fn a_changed_shape_is_a_different_question() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let cache = Cache::new(&db.conn);
        let queries = vec!["Dune".to_string()];

        cache
            .put("wikidata", "label", "shape-a", "Dune", &answer(Outcome::Empty, None))
            .await
            .expect("put should succeed");

        assert!(cache.get_many("wikidata", "label", "shape-a", &queries).await.unwrap().contains_key("Dune"));
        assert!(
            cache.get_many("wikidata", "label", "shape-b", &queries).await.unwrap().is_empty(),
            "a stale negative must not outlive the query shape that produced it"
        );
    }

    #[tokio::test]
    async fn refetching_replaces_instead_of_accumulating() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let cache = Cache::new(&db.conn);
        let queries = vec!["Nintendo".to_string()];

        cache
            .put("wikidata", "label", "v1", "Nintendo", &answer(Outcome::Empty, None))
            .await
            .expect("put should succeed");
        cache
            .put("wikidata", "label", "v1", "Nintendo", &answer(Outcome::Found, Some(json!({"qid": "Q1"}))))
            .await
            .expect("second put should succeed");

        let hits = cache.get_many("wikidata", "label", "v1", &queries).await.expect("get");
        assert_eq!(hits["Nintendo"].outcome, Outcome::Found);
        assert_eq!(
            cached::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "one row per logical request"
        );
    }
}
