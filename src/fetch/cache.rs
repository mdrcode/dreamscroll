//! Cache by default: the only door to a third-party provider.
//!
//! Every request a caller makes passes through here first. A warm cache means no network call at
//! all, and a *negative* answer is cached as deliberately as a positive one — the tail is a third of
//! any corpus, and re-asking a question we have already answered is the spam this exists to prevent.
//!
//! The table is one row per logical request, and the payload is the provider's own type, serialised.
//! Nothing here interprets it: a payload that no longer decodes is a **miss**, not an empty answer,
//! so a changed shape of answer is re-asked rather than silently believed.

use std::collections::HashMap;
use std::fmt::Display;

use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::transport::{Answer, Outcome};
use crate::model::fetch_cache as cached;

/// Seven days for everything, which is short enough that a provider gaining a new item is noticed
/// and long enough that a rerun is free.
pub const DEFAULT_TTL: Duration = Duration::days(7);

/// The one exception: a provider that could not be reached is not a fact about the world, and
/// caching "the provider was down" for a week would poison that part of the corpus.
pub const FAILED_TTL: Duration = Duration::minutes(10);

/// Longer than [`FAILED_TTL`] because a degraded answer is usually still true — it is just not the
/// answer we would have got from a healthy backend. One search API silently served prefix-only
/// results when its index was unavailable, with HTTP 200 and `success: 1`; a week of that would
/// quarantine a query behind a bad answer nobody could see was bad.
pub const DEGRADED_TTL: Duration = Duration::minutes(60);

/// Expired rows are kept this long past their expiry before being pruned. Long enough that recent
/// staleness stays inspectable — "we asked last week and the answer has aged out" is worth seeing —
/// and short enough that the table does not become an archive of answers nothing reads.
pub const PRUNE_GRACE: Duration = Duration::days(30);

/// The policy a row is fetched under, per outcome.
fn ttl(outcome: Outcome) -> Duration {
    match outcome {
        Outcome::Failed => FAILED_TTL,
        Outcome::Degraded => DEGRADED_TTL,
        Outcome::Found | Outcome::Empty => DEFAULT_TTL,
    }
}

/// A stored payload, decoded. `None` when the answer carried nothing, or when the payload no longer
/// decodes into what the caller expects — a changed payload shape is a new question, not a body to
/// read.
pub fn decode<P: DeserializeOwned>(payload: Option<&Value>) -> Option<P> {
    payload.and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Where a stored answer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Present and inside its TTL: the resolver would use it.
    Fresh,
    /// Present but past its TTL: a miss to the resolver, still inspectable.
    Expired,
    /// No row for this request.
    Missing,
}

/// One request's stored answer, with the freshness a caller needs to decide what to do with it.
pub struct Lookup<P> {
    pub state: State,
    /// The stored outcome; `None` only when no row exists.
    pub outcome: Option<Outcome>,
    /// The decoded payload. `None` for a negative, or when the payload no longer reads.
    pub body: Option<P>,
    /// An answered row whose payload does not decode into `P`: a miss, never an empty answer.
    pub unreadable: bool,
}

/// What one stored row means, before freshness is applied.
struct Stored<P> {
    outcome: Outcome,
    body: Option<P>,
    unreadable: bool,
}

/// One interpretation of a row, shared by [`FetchCache::get_many`] and [`FetchCache::lookup`].
fn interpret<P: DeserializeOwned>(row: &cached::Model) -> Stored<P> {
    let body = decode(row.payload.as_ref());
    Stored {
        unreadable: row.outcome.answered() && row.payload.is_some() && body.is_none(),
        outcome: row.outcome,
        body,
    }
}

pub struct FetchCache<'a> {
    conn: &'a DatabaseConnection,
}

impl<'a> FetchCache<'a> {
    pub fn new(conn: &'a DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Drop answers that expired more than [`PRUNE_GRACE`] ago.
    ///
    /// A stale row is invisible to [`Self::get_many`] and is only ever rewritten if the same query is
    /// asked again, so without this the table grows monotonically with every query ever made and holds
    /// answers nothing can read.
    pub async fn prune(&self) -> anyhow::Result<u64> {
        let cutoff = Utc::now() - PRUNE_GRACE;
        let result = cached::Entity::delete_many()
            .filter(cached::Column::ExpiresAt.lt(cutoff))
            .exec(self.conn)
            .await?;
        Ok(result.rows_affected)
    }

    /// Answers for the queries that are still fresh. Anything absent is a miss, including rows that
    /// exist but have expired: a stale answer is not an answer.
    pub async fn get_many<K: Display, P: DeserializeOwned>(
        &self,
        provider: &str,
        kind: &K,
        shape: &str,
        queries: &[String],
    ) -> anyhow::Result<HashMap<String, Answer<P>>> {
        if queries.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = cached::Entity::find()
            .filter(cached::Column::Provider.eq(provider))
            .filter(cached::Column::Kind.eq(kind.to_string()))
            .filter(cached::Column::Shape.eq(shape))
            .filter(cached::Column::Query.is_in(queries.iter().cloned()))
            .filter(cached::Column::ExpiresAt.gt(Utc::now()))
            .all(self.conn)
            .await?;

        let mut answers = HashMap::with_capacity(rows.len());
        for row in rows {
            let stored = interpret::<P>(&row);
            // An answered row whose payload no longer reads is not an answer. Let the request miss
            // and be asked again, rather than handing back a body nothing can use.
            if stored.unreadable {
                tracing::warn!(
                    provider,
                    kind = %kind,
                    query = %row.query,
                    "unreadable cached payload; treating it as a miss"
                );
                continue;
            }
            answers.insert(
                row.query,
                Answer {
                    outcome: stored.outcome,
                    body: stored.body,
                },
            );
        }
        Ok(answers)
    }

    /// One request's stored answer, fresh or stale, so a caller can show *why* a rung refused.
    ///
    /// [`Self::get_many`] is the resolver's door: it drops everything not fresh. This is the
    /// inspector's: a stale row is still evidence of what was asked and what came back.
    pub async fn lookup<K: Display, P: DeserializeOwned>(
        &self,
        provider: &str,
        kind: &K,
        shape: &str,
        query: &str,
    ) -> anyhow::Result<Lookup<P>> {
        let row = cached::Entity::find()
            .filter(cached::Column::Provider.eq(provider))
            .filter(cached::Column::Kind.eq(kind.to_string()))
            .filter(cached::Column::Shape.eq(shape))
            .filter(cached::Column::Query.eq(query))
            .one(self.conn)
            .await?;
        Ok(match row {
            None => Lookup {
                state: State::Missing,
                outcome: None,
                body: None,
                unreadable: false,
            },
            Some(row) => {
                let stored = interpret::<P>(&row);
                Lookup {
                    state: if row.expires_at > Utc::now() {
                        State::Fresh
                    } else {
                        State::Expired
                    },
                    outcome: Some(stored.outcome),
                    body: stored.body,
                    unreadable: stored.unreadable,
                }
            }
        })
    }

    /// Record an answer. One row per logical request: an expired or re-fetched answer replaces the
    /// old one rather than accumulating history, because the payload is a cache, not evidence.
    pub async fn put<K: Display, P: Serialize>(
        &self,
        provider: &str,
        kind: &K,
        shape: &str,
        query: &str,
        answer: &Answer<P>,
    ) -> anyhow::Result<()> {
        let expires_at: DateTime<Utc> = Utc::now() + ttl(answer.outcome);
        let payload = match &answer.body {
            Some(body) => Some(serde_json::to_value(body)?),
            None => None,
        };
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
            outcome: Set(answer.outcome),
            payload: Set(payload),
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
    use crate::test_support::db_or_skip;

    /// The payload is the provider's own type; the cache never looks inside it.
    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Record {
        id: String,
    }

    fn record(id: &str) -> Record {
        Record { id: id.to_string() }
    }

    /// A degraded answer is still parsed — it may be right — but it must not stand for a week.
    #[test]
    fn a_degraded_answer_expires_sooner_than_a_found_one() {
        assert!(Outcome::Degraded.answered());
        assert!(Outcome::Found.answered());
        assert!(!Outcome::Empty.answered());
        assert!(!Outcome::Failed.answered());
        assert!(ttl(Outcome::Degraded) < ttl(Outcome::Found));
        assert!(ttl(Outcome::Degraded) > ttl(Outcome::Failed));
        assert_eq!(Outcome::parse("degraded"), Some(Outcome::Degraded));
    }

    /// Pruning drops what aged out long ago and keeps recent staleness inspectable: an answer that
    /// expired yesterday is still worth seeing, one from two months ago is an archive nothing reads.
    #[tokio::test]
    async fn pruning_keeps_the_recent_past_and_drops_the_ancient() {
        db_or_skip!(db);
        let conn = db.conn;
        let insert = |query: &'static str, expires_at: chrono::DateTime<Utc>| {
            let conn = conn.clone();
            async move {
                cached::Entity::insert(cached::ActiveModel {
                    provider: Set("probe".to_string()),
                    kind: Set("kind-a".to_string()),
                    query: Set(query.to_string()),
                    shape: Set("shape-a".to_string()),
                    outcome: Set(Outcome::Found),
                    payload: Set(Some(json!([{"id": "one"}]))),
                    expires_at: Set(expires_at),
                    ..Default::default()
                })
                .exec(&conn)
                .await
                .unwrap();
            }
        };
        let now = Utc::now();
        insert("ancient", now - PRUNE_GRACE - Duration::days(1)).await;
        insert("recently expired", now - Duration::days(1)).await;
        insert("live", now + Duration::days(1)).await;

        let pruned = FetchCache::new(&conn).prune().await.expect("prune");

        assert_eq!(pruned, 1, "only the row past the grace window goes");
        let left: Vec<String> = cached::Entity::find()
            .all(&conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.query)
            .collect();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(!left.iter().any(|q| q == "ancient"), "{left:?}");
    }

    async fn put(cache: &FetchCache<'_>, shape: &str, query: &str, outcome: Outcome) {
        cache
            .put("probe", &"kind-a", shape, query, &Answer::<Vec<Record>>::nothing(outcome))
            .await
            .expect("put should succeed");
    }

    #[tokio::test]
    async fn stores_reads_and_expires() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let queries = vec!["first query".to_string(), "second query".to_string()];

        cache
            .put(
                "probe",
                &"kind-a",
                "v1",
                "first query",
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(vec![record("Q1")]),
                },
            )
            .await
            .expect("put should succeed");
        put(&cache, "v1", "second query", Outcome::Empty).await;

        let hits: HashMap<String, Answer<Vec<Record>>> = cache
            .get_many("probe", &"kind-a", "v1", &queries)
            .await
            .expect("get");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits["first query"].outcome, Outcome::Found);
        assert_eq!(
            hits["first query"].body,
            Some(vec![record("Q1")]),
            "the payload survives the round trip"
        );
        assert_eq!(
            hits["second query"].outcome,
            Outcome::Empty,
            "a negative is a cached answer"
        );
        assert_eq!(hits["second query"].body, None);

        // Expire one row by hand; a stale answer must not be served.
        cached::Entity::update_many()
            .col_expr(
                cached::Column::ExpiresAt,
                sea_orm::sea_query::Expr::value(Utc::now() - Duration::seconds(1)),
            )
            .filter(cached::Column::Query.eq("second query"))
            .exec(&db.conn)
            .await
            .expect("expiry update should succeed");

        let hits: HashMap<String, Answer<Vec<Record>>> = cache
            .get_many("probe", &"kind-a", "v1", &queries)
            .await
            .expect("get");
        assert_eq!(hits.len(), 1, "the expired row is a miss: {hits:?}");
        assert!(!hits.contains_key("second query"));
    }

    /// A payload the caller can no longer read is not an answer. It has to miss, or a provider's
    /// changed payload shape would be served as an empty result for the rest of the TTL.
    #[tokio::test]
    async fn an_unreadable_payload_is_a_miss() {
        db_or_skip!(db);
        let conn = db.conn;
        cached::Entity::insert(cached::ActiveModel {
            provider: Set("probe".to_string()),
            kind: Set("kind-a".to_string()),
            query: Set("shape changed".to_string()),
            shape: Set("v1".to_string()),
            outcome: Set(Outcome::Found),
            payload: Set(Some(json!({"an": "envelope nothing knows"}))),
            expires_at: Set(Utc::now() + Duration::days(1)),
            ..Default::default()
        })
        .exec(&conn)
        .await
        .unwrap();

        let hits: HashMap<String, Answer<Vec<Record>>> = FetchCache::new(&conn)
            .get_many("probe", &"kind-a", "v1", &["shape changed".to_string()])
            .await
            .expect("get");

        assert!(
            hits.is_empty(),
            "an undecodable body must come back as a miss: {hits:?}"
        );
    }

    #[tokio::test]
    async fn a_changed_shape_is_a_different_question() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let queries = vec!["query one".to_string()];

        put(&cache, "shape-a", "query one", Outcome::Empty).await;

        let hits: HashMap<String, Answer<Vec<Record>>> = cache
            .get_many("probe", &"kind-a", "shape-a", &queries)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "the row is there, asked for the right way");
        let hits: HashMap<String, Answer<Vec<Record>>> = cache
            .get_many("probe", &"kind-a", "shape-b", &queries)
            .await
            .unwrap();
        assert!(
            hits.is_empty(),
            "a stale negative must not outlive the query shape that produced it"
        );
    }

    #[tokio::test]
    async fn refetching_replaces_instead_of_accumulating() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        let queries = vec!["query one".to_string()];

        put(&cache, "v1", "query one", Outcome::Empty).await;
        put(&cache, "v1", "query one", Outcome::Found).await;

        let hits: HashMap<String, Answer<Vec<Record>>> = cache
            .get_many("probe", &"kind-a", "v1", &queries)
            .await
            .expect("get");
        assert_eq!(hits["query one"].outcome, Outcome::Found);
        assert_eq!(
            cached::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "one row per logical request"
        );
    }

    /// The four states an inspector must tell apart; `get_many` collapses three of them to a miss.
    #[tokio::test]
    async fn lookup_separates_fresh_expired_missing_and_unreadable() {
        db_or_skip!(db);
        let cache = FetchCache::new(&db.conn);
        cache
            .put(
                "probe",
                &"kind-a",
                "shape-a",
                "fresh",
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(vec![record("one")]),
                },
            )
            .await
            .unwrap();
        let insert = |query: &'static str, expires_at: DateTime<Utc>, payload: Option<Value>| {
            let conn = db.conn.clone();
            async move {
                cached::Entity::insert(cached::ActiveModel {
                    provider: Set("probe".to_string()),
                    kind: Set("kind-a".to_string()),
                    query: Set(query.to_string()),
                    shape: Set("shape-a".to_string()),
                    outcome: Set(Outcome::Found),
                    payload: Set(payload),
                    expires_at: Set(expires_at),
                    ..Default::default()
                })
                .exec(&conn)
                .await
                .unwrap();
            }
        };
        insert(
            "expired",
            Utc::now() - Duration::minutes(1),
            Some(json!([{"id": "two"}])),
        )
        .await;
        insert(
            "unreadable",
            Utc::now() + Duration::days(1),
            Some(json!({"not": "a list"})),
        )
        .await;

        let fresh = cache
            .lookup::<_, Vec<Record>>("probe", &"kind-a", "shape-a", "fresh")
            .await
            .unwrap();
        assert_eq!(fresh.state, State::Fresh);
        assert_eq!(fresh.outcome, Some(Outcome::Found));
        assert_eq!(fresh.body, Some(vec![record("one")]));
        assert!(!fresh.unreadable);

        let expired = cache
            .lookup::<_, Vec<Record>>("probe", &"kind-a", "shape-a", "expired")
            .await
            .unwrap();
        assert_eq!(expired.state, State::Expired);
        assert_eq!(expired.body, Some(vec![record("two")]));

        let missing = cache
            .lookup::<_, Vec<Record>>("probe", &"kind-a", "shape-a", "missing")
            .await
            .unwrap();
        assert_eq!(missing.state, State::Missing);
        assert_eq!(missing.outcome, None);
        assert!(missing.body.is_none());

        let unreadable = cache
            .lookup::<_, Vec<Record>>("probe", &"kind-a", "shape-a", "unreadable")
            .await
            .unwrap();
        assert_eq!(unreadable.state, State::Fresh);
        assert!(unreadable.unreadable);
        assert!(unreadable.body.is_none());
    }
}
