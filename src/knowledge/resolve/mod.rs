//! Resolution: ask providers about cores, through the cache, and never any other way.
//!
//! The rule this module enforces structurally: **only the executor calls an engine's `fetch`**. A
//! provider may say what it would ask (`plan`) and interpret an answer (`parse`), but it cannot
//! reach the network on its own — every request goes through the cache first. Adding IMDb means
//! adding one `Engine`; it cannot forget the cache, because there is no other door.

pub mod wikidata;

use std::collections::{BTreeMap, HashMap, HashSet};

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set};
use serde_json::{json, Value};

use super::cache::{Answer, Cache, Outcome};
use crate::{database, model};

/// How many queries go into one transport call. Providers that cannot batch ignore the chunking and
/// answer one request per query; the executor does not care.
#[cfg(not(test))]
pub(crate) const BATCH: usize = 50;

/// Smaller under test, so a second transport call — where the rate limit is exercised — is reachable
/// with a handful of cores.
#[cfg(test)]
pub(crate) const BATCH: usize = 2;

/// What a provider is allowed to see about a core. Deliberately small: a provider gets the surface,
/// the type, the text and its identifiers, and nothing about who captured it.
#[derive(Debug, Clone)]
pub struct CoreView {
    pub id: i32,
    pub label: String,
    pub entity_type: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    /// The best disambiguation hint an observation carried, if any.
    pub search_query: Option<String>,
    /// `(platform, handle)` when one of the core's observations carried an account.
    pub handle: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub provider: &'static str,
    pub kind: String,
    /// Hash of how the request is built; part of the cache key, so a changed query is a new question.
    pub shape: String,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub outcome: Outcome,
    pub payload: Option<Value>,
}

/// One identity a provider is willing to assert about a core.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub scheme: String,
    pub id: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub method: String,
    pub confidence: f32,
}

/// What went wrong at the transport. The distinction matters: a `Failed` request is a fact about the
/// provider ("we could not reach it") and may be cached briefly, while a `RateLimited` response is an
/// instruction — stop, and come back after `Retry-After`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    RateLimited { retry_after_secs: u64 },
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::RateLimited { retry_after_secs } => {
                write!(f, "rate limited, retry after {retry_after_secs}s")
            }
            FetchError::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for FetchError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Identifiers and exact labels: cheap, high precision.
    Identity,
    /// Full-text search: only for cores the first stage could not resolve.
    Fallback,
}

#[async_trait::async_trait]
pub trait Engine: Send + Sync {
    fn id(&self) -> &'static str;

    /// What this engine would ask about a core. Empty when it has nothing to offer.
    fn plan(&self, core: &CoreView) -> Vec<Request>;

    /// Second-stage questions, asked only when the first stage found nothing.
    fn plan_fallback(&self, _core: &CoreView) -> Vec<Request> {
        Vec::new()
    }

    /// Turn an answer into claims. Pure: same core plus same answer, same claims. The core is
    /// passed because deciding between ambiguous candidates needs its description and type.
    fn parse(&self, core: &CoreView, request: &Request, raw: &Raw) -> Vec<Claim>;

    /// How many queries this kind can answer in one network call. The executor chunks by this and
    /// paces each call, so a kind that cannot batch is throttled per query instead of fired as a
    /// burst — which is how the search endpoint got us rate limited.
    fn batch_size(&self, _kind: &str) -> usize {
        BATCH
    }

    /// The only network call in the system. `requests` are all the same kind and shape.
    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError>;
}

#[derive(Debug, Clone)]
pub struct Options {
    /// How many candidate cores to consider.
    pub limit: Option<usize>,
    /// Transport calls allowed in this run — the anti-spam budget.
    pub max_requests: usize,
    /// Minimum wall-clock gap between transport calls. One request per second keeps a walk of a few
    /// thousand labels to a few minutes while staying far below WDQS's processing-time budget.
    pub min_interval: std::time::Duration,
    pub dry_run: bool,
    /// Resolve only this core — for debugging one case without walking the corpus.
    pub only: Option<i32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            limit: None,
            max_requests: 200,
            min_interval: std::time::Duration::from_secs(1),
            dry_run: false,
            only: None,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub candidates: usize,
    pub planned: usize,
    pub cache_hits: usize,
    /// Transport calls made.
    pub fetched: usize,
    pub failed: usize,
    pub claims: usize,
    /// Cores that gained at least one claim.
    pub cores_resolved: usize,
    /// Cores whose identity is already claimed by another core — a merge the graph has not made.
    pub conflicts: usize,
    pub budget_exhausted: bool,
    /// Set when the provider asked us to back off: the run stopped instead of risking a ban.
    pub rate_limited_after: Option<u64>,
    pub dry_run: bool,
}

pub async fn resolve(
    db: &database::DbHandle,
    engines: &[Box<dyn Engine>],
    options: &Options,
) -> anyhow::Result<Report> {
    let cores = candidates(&db.conn, options.limit, options.only).await?;
    let cache = Cache::new(&db.conn);
    let mut report = Report {
        candidates: cores.len(),
        dry_run: options.dry_run,
        ..Default::default()
    };
    let mut resolved: HashSet<i32> = HashSet::new();

    for engine in engines {
        if report.rate_limited_after.is_some() {
            break;
        }
        run_stage(db, &cache, engine.as_ref(), &cores, &mut resolved, Stage::Identity, options, &mut report).await?;
    }
    // Stage two exists for providers whose second request is worth making. Wikidata's is not: its
    // entity search is a label matcher, so `plan_fallback` returns nothing.
    if !options.dry_run {
        for engine in engines {
            if report.rate_limited_after.is_some() {
                break;
            }
            run_stage(db, &cache, engine.as_ref(), &cores, &mut resolved, Stage::Fallback, options, &mut report).await?;
        }
    }

    for core_id in &resolved {
        project(&db.conn, *core_id).await?;
    }
    report.cores_resolved = resolved.len();
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
async fn run_stage(
    db: &database::DbHandle,
    cache: &Cache<'_>,
    engine: &dyn Engine,
    cores: &[CoreView],
    resolved: &mut HashSet<i32>,
    stage: Stage,
    options: &Options,
    report: &mut Report,
) -> anyhow::Result<()> {
    let plans: Vec<(usize, Request)> = cores
        .iter()
        .enumerate()
        .filter(|(_, core)| !resolved.contains(&core.id))
        .flat_map(|(index, core)| {
            let requests = match stage {
                Stage::Identity => engine.plan(core),
                Stage::Fallback => engine.plan_fallback(core),
            };
            requests.into_iter().map(move |request| (index, request))
        })
        .collect();
    if plans.is_empty() {
        return Ok(());
    }
    report.planned += plans.len();

    // Note the order: a rate limit stops *further fetching*, not the parsing of answers already in
    // hand. Returning here discarded every claim from the batches that succeeded.
    let answers = fetch_plans(cache, engine, &plans, options, report).await?;

    for (index, request) in plans {
        let Some(raw) = answers.get(&request.query) else {
            continue;
        };
        let claims = engine.parse(&cores[index], &request, raw);
        if claims.is_empty() {
            continue;
        }
        let core_id = cores[index].id;
        let mut accepted = 0;
        for claim in claims {
            if store_claim(&db.conn, engine.id(), core_id, claim, report).await? {
                accepted += 1;
            }
        }
        if accepted > 0 {
            resolved.insert(core_id);
        }
    }
    Ok(())
}

/// Look every planned query up in the cache, then fetch only what is missing, in batches.
async fn fetch_plans(
    cache: &Cache<'_>,
    engine: &dyn Engine,
    plans: &[(usize, Request)],
    options: &Options,
    report: &mut Report,
) -> anyhow::Result<HashMap<String, Raw>> {
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (_, request) in plans {
        groups
            .entry((request.kind.clone(), request.shape.clone()))
            .or_default()
            .push(request.query.clone());
    }
    for queries in groups.values_mut() {
        queries.sort();
        queries.dedup();
    }

    let mut answers: HashMap<String, Raw> = HashMap::new();
    let mut last_call: Option<std::time::Instant> = None;

    for ((kind, shape), queries) in groups {
        let hits = cache.get_many(engine.id(), &kind, &shape, &queries).await?;
        report.cache_hits += hits.len();
        for (query, answer) in hits {
            answers.insert(
                query,
                Raw {
                    outcome: answer.outcome,
                    payload: answer.payload,
                },
            );
        }

        let misses: Vec<String> = queries
            .into_iter()
            .filter(|query| !answers.contains_key(query))
            .collect();
        if misses.is_empty() || options.dry_run {
            continue;
        }

        for chunk in misses.chunks(engine.batch_size(&kind)) {
            if report.fetched >= options.max_requests {
                report.budget_exhausted = true;
                break;
            }
            let requests: Vec<Request> = chunk
                .iter()
                .map(|query| Request {
                    provider: engine.id(),
                    kind: kind.clone(),
                    shape: shape.clone(),
                    query: query.clone(),
                })
                .collect();
            // Space calls out. WDQS meters processing time, not request count, but a floor is the
            // simplest thing that cannot be wrong.
            if let Some(previous) = last_call {
                let ready = previous + options.min_interval;
                let now = std::time::Instant::now();
                if ready > now {
                    tokio::time::sleep(ready - now).await;
                }
            }
            last_call = Some(std::time::Instant::now());
            report.fetched += 1;

            match engine.fetch(&requests).await {
                Ok(results) => {
                    for (query, raw) in results {
                        cache
                            .put(
                                engine.id(),
                                &kind,
                                &shape,
                                &query,
                                &Answer {
                                    outcome: raw.outcome,
                                    payload: raw.payload.clone(),
                                },
                            )
                            .await?;
                        answers.insert(query, raw);
                    }
                }
                Err(FetchError::RateLimited { retry_after_secs }) => {
                    // An instruction, not a fact. Do not cache it, do not retry, do not continue:
                    // WDQS temporarily bans clients that keep requesting after a 429.
                    report.rate_limited_after = Some(retry_after_secs);
                    return Ok(answers);
                }
                Err(FetchError::Failed(_)) => {
                    // Not a fact about the world either: cache it briefly so a sick provider is not
                    // hammered, then move on.
                    for query in chunk {
                        cache
                            .put(
                                engine.id(),
                                &kind,
                                &shape,
                                query,
                                &Answer {
                                    outcome: Outcome::Failed,
                                    payload: None,
                                },
                            )
                            .await?;
                        answers.insert(
                            query.clone(),
                            Raw {
                                outcome: Outcome::Failed,
                                payload: None,
                            },
                        );
                        report.failed += 1;
                    }
                }
            }
        }
    }

    Ok(answers)
}

/// Store one claim as `ontology` evidence. Returns whether it was accepted; a claim another core
/// already holds is a conflict (a merge the graph has not made), not a duplicate.
async fn store_claim(
    conn: &sea_orm::DatabaseConnection,
    source: &str,
    core_id: i32,
    claim: Claim,
    report: &mut Report,
) -> anyhow::Result<bool> {
    let value = json!({
        "scheme": claim.scheme,
        "id": claim.id,
        "label": claim.label,
        "description": claim.description,
        "classes": claim.classes,
    });
    let insert = model::core_knowledge_evidence::Entity::insert(
        model::core_knowledge_evidence::ActiveModel {
            core_id: Set(core_id),
            kind: Set("ontology".to_string()),
            value: Set(value),
            source: Set(source.to_string()),
            method: Set(Some(claim.method)),
            confidence: Set(Some(claim.confidence)),
            ..Default::default()
        },
    )
    .exec(conn)
    .await;

    match insert {
        Ok(_) => {
            report.claims += 1;
            Ok(true)
        }
        Err(error) => match unique_violation(&error) {
            Some("idx-core_evidence_identity") => {
                report.conflicts += 1;
                Ok(false)
            }
            // The same fact already recorded for this core: nothing to do.
            Some(_) => Ok(false),
            None => Err(error.into()),
        },
    }
}

fn unique_violation(error: &sea_orm::DbErr) -> Option<&str> {
    use sea_orm::RuntimeErr;
    match error {
        sea_orm::DbErr::Exec(RuntimeErr::SqlxError(error))
        | sea_orm::DbErr::Query(RuntimeErr::SqlxError(error)) => error
            .as_database_error()
            .filter(|error| error.code().as_deref() == Some("23505"))
            .and_then(|error| error.constraint()),
        _ => None,
    }
}

/// Recompute the columns that are a function of evidence. `status` is not written anywhere else.
pub async fn project(conn: &sea_orm::DatabaseConnection, core_id: i32) -> anyhow::Result<()> {
    use model::core_knowledge_evidence as evidence;

    let claims = evidence::Entity::find()
        .filter(evidence::Column::CoreId.eq(core_id))
        .filter(evidence::Column::Kind.is_in(["ontology", "platform_link"]))
        .filter(evidence::Column::RetractedAt.is_null())
        .all(conn)
        .await?;
    let contested = evidence::Entity::find()
        .filter(evidence::Column::CoreId.eq(core_id))
        .filter(evidence::Column::Kind.eq("collision"))
        .filter(evidence::Column::RetractedAt.is_null())
        .count(conn)
        .await?
        > 0;

    let status = if contested {
        "contested"
    } else if claims.is_empty() {
        "provisional"
    } else {
        "identified"
    };

    // Corroboration: each additional *independent scheme* agreeing raises confidence. Two authorities
    // asserting the same referent is a stronger claim than either alone.
    // Corroboration counts independent *inputs*, not scheme labels. A `p2002` claim is derived from
    // the handle — the resolver asked Wikidata which item owns that account — so counting it beside
    // the handle's own `platform_link` row treated one chain as two and gave every handle-resolved
    // core the maximum confidence.
    let from_text = claims
        .iter()
        .any(|claim| claim.kind == "ontology" && claim.method.as_deref() != Some("p2002"));
    let from_handle = claims.iter().any(|claim| claim.kind == "platform_link");
    let inputs = usize::from(from_text) + usize::from(from_handle);

    let mut best = 0.0f32;
    for claim in &claims {
        best = best.max(claim.confidence.unwrap_or(0.0));
    }
    let corroboration = inputs.saturating_sub(1) as f32 * 0.1;
    let confidence = if claims.is_empty() {
        None
    } else {
        Some((best + corroboration).min(1.0))
    };

    model::core_knowledge_entity::Entity::update_many()
        .col_expr(
            model::core_knowledge_entity::Column::Status,
            Expr::value(status),
        )
        .col_expr(
            model::core_knowledge_entity::Column::Confidence,
            Expr::value(confidence),
        )
        .filter(model::core_knowledge_entity::Column::Id.eq(core_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// Cores with no live ontology claim. Note the filter is *ontology*, not *no evidence*: a core
/// identified only by a platform handle still deserves a QID, because that is how corroboration
/// happens.
async fn candidates(
    conn: &sea_orm::DatabaseConnection,
    limit: Option<usize>,
    only: Option<i32>,
) -> anyhow::Result<Vec<CoreView>> {
    use model::core_knowledge_entity as core;
    use model::core_knowledge_evidence as evidence;
    use model::user_knowledge_entity as observation;

    let already: HashSet<i32> = evidence::Entity::find()
        .filter(evidence::Column::Kind.eq("ontology"))
        .filter(evidence::Column::RetractedAt.is_null())
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.core_id)
        .collect();

    let mut cores = core::Entity::find()
        .filter(core::Column::SupersededById.is_null())
        .order_by_asc(core::Column::Id)
        .all(conn)
        .await?;
    cores.retain(|row| !already.contains(&row.id));
    if let Some(core_id) = only {
        cores.retain(|row| row.id == core_id);
    }
    if let Some(limit) = limit {
        cores.truncate(limit);
    }

    let observations = observation::Entity::find()
        .filter(observation::Column::CoreId.is_not_null())
        .all(conn)
        .await?;
    let mut search: HashMap<i32, String> = HashMap::new();
    let mut handles: HashMap<i32, (String, String)> = HashMap::new();
    for row in observations {
        let Some(core_id) = row.core_id else { continue };
        if let Some(query) = row.search_query {
            let query = query.trim().to_string();
            if !query.is_empty() {
                let entry = search.entry(core_id).or_default();
                if query.len() > entry.len() {
                    *entry = query;
                }
            }
        }
        if let (Some(platform), Some(handle)) = (row.platform, row.platform_handle) {
            handles.entry(core_id).or_insert((platform, handle));
        }
    }

    Ok(cores
        .into_iter()
        .map(|row| CoreView {
            search_query: search.get(&row.id).cloned(),
            handle: handles.get(&row.id).cloned(),
            id: row.id,
            label: row.label,
            entity_type: row.entity_type,
            description: row.description,
            aliases: row.aliases,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    /// An engine that never touches the network: it answers from a fixed map and counts calls.
    struct Fake {
        answers: HashMap<String, Raw>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Engine for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }

        fn plan(&self, core: &CoreView) -> Vec<Request> {
            vec![Request {
                provider: "fake",
                kind: "label".to_string(),
                shape: "v1".to_string(),
                query: core.label.clone(),
            }]
        }

        fn parse(&self, _core: &CoreView, request: &Request, raw: &Raw) -> Vec<Claim> {
            let Some(payload) = &raw.payload else {
                return Vec::new();
            };
            payload
                .get("claim")
                .and_then(Value::as_str)
                .map(|id| Claim {
                    scheme: "fake".to_string(),
                    id: id.to_string(),
                    label: Some(request.query.clone()),
                    description: None,
                    classes: Vec::new(),
                    method: "exact_label".to_string(),
                    confidence: 0.9,
                })
                .into_iter()
                .collect()
        }

        async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(requests
                .iter()
                .map(|request| {
                    let raw = self
                        .answers
                        .get(&request.query)
                        .cloned()
                        .unwrap_or(Raw {
                            outcome: Outcome::Empty,
                            payload: None,
                        });
                    (request.query.clone(), raw)
                })
                .collect())
        }
    }

    /// One observation, linked to a core by the real linker — the same path production takes.
    async fn seed(db: &database::DbHandle, name: &str) -> i32 {
        use crate::test_support::fixtures;

        let user_id = fixtures::user(db, "resolve").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let entity_id = model::user_knowledge_entity::Entity::insert(
            model::user_knowledge_entity::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(0),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                ..Default::default()
            },
        )
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;

        crate::knowledge::link::build_core(db).await.unwrap();

        model::user_knowledge_entity::Entity::find_by_id(entity_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .core_id
            .expect("build_core should have linked the observation")
    }

    fn fake(answers: HashMap<String, Raw>, calls: Arc<AtomicUsize>) -> Vec<Box<dyn Engine>> {
        vec![Box::new(Fake { answers, calls })]
    }

    #[tokio::test]
    async fn a_claim_identifies_the_core_and_a_second_run_asks_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let core_id = seed(&db, "Train Dreams").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let engines = fake(
            HashMap::from([(
                "Train Dreams".to_string(),
                Raw {
                    outcome: Outcome::Found,
                    payload: Some(json!({"claim": "Q185971"})),
                },
            )]),
            calls.clone(),
        );

        let first = resolve(&db, &engines, &Options::default()).await.expect("resolve");
        assert_eq!(first.claims, 1, "{first:?}");
        assert_eq!(first.fetched, 1, "{first:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let core = model::core_knowledge_entity::Entity::find_by_id(core_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(core.status, "identified");
        assert_eq!(core.confidence, Some(0.9));

        // Second run: the core now has an ontology claim, so it is not even a candidate.
        let second = resolve(&db, &engines, &Options::default()).await.expect("resolve");
        assert_eq!(second.candidates, 0, "{second:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second fetch");
    }

    #[tokio::test]
    async fn an_empty_answer_is_cached_and_not_re_asked() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let core_id = seed(&db, "MUME").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let engines = fake(HashMap::new(), calls.clone());

        let first = resolve(&db, &engines, &Options::default()).await.expect("resolve");
        assert_eq!(first.fetched, 1, "{first:?}");
        assert_eq!(first.claims, 0);

        // The core is still a candidate (no claim), but the negative answer is cached.
        let second = resolve(&db, &engines, &Options::default()).await.expect("resolve");
        assert_eq!(second.candidates, 1, "{second:?}");
        assert_eq!(second.fetched, 0, "{second:?}");
        assert_eq!(second.cache_hits, 1, "{second:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the negative was not re-fetched");

        let core = model::core_knowledge_entity::Entity::find_by_id(core_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(core.status, "provisional");
    }

    /// Two *independent inputs* — a text match and an account link — raise confidence.
    #[tokio::test]
    async fn corroboration_counts_independent_inputs() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let core_id = seed(&db, "Nintendo").await;

        model::core_knowledge_evidence::Entity::insert(
            model::core_knowledge_evidence::ActiveModel {
                core_id: Set(core_id),
                kind: Set("ontology".to_string()),
                value: Set(json!({"scheme": "wikidata", "id": "Q8093"})),
                source: Set("wikidata".to_string()),
                method: Set(Some("exact_label".to_string())),
                confidence: Set(Some(0.8)),
                ..Default::default()
            },
        )
        .exec(&db.conn)
        .await
        .unwrap();
        model::core_knowledge_evidence::Entity::insert(
            model::core_knowledge_evidence::ActiveModel {
                core_id: Set(core_id),
                kind: Set("platform_link".to_string()),
                value: Set(json!({"scheme": "x_twitter", "id": "nintendo"})),
                source: Set("platform".to_string()),
                ..Default::default()
            },
        )
        .exec(&db.conn)
        .await
        .unwrap();

        project(&db.conn, core_id).await.unwrap();

        let core = model::core_knowledge_entity::Entity::find_by_id(core_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!((core.confidence.unwrap() - 0.9).abs() < 1e-6, "{core:?}");
    }

    /// A `p2002` claim *is* the handle — it must not count as a second input beside it.
    #[tokio::test]
    async fn a_handle_derived_claim_is_not_corroboration() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let core_id = seed(&db, "Tsoding").await;

        for (kind, method, value, confidence) in [
            ("platform_link", None, json!({"scheme": "x_twitter", "id": "tsoding"}), None),
            ("ontology", Some("p2002"), json!({"scheme": "wikidata", "id": "Q123737256"}), Some(0.95)),
        ] {
            model::core_knowledge_evidence::Entity::insert(
                model::core_knowledge_evidence::ActiveModel {
                    core_id: Set(core_id),
                    kind: Set(kind.to_string()),
                    value: Set(value),
                    source: Set("test".to_string()),
                    method: Set(method.map(str::to_string)),
                    confidence: Set(confidence),
                    ..Default::default()
                },
            )
            .exec(&db.conn)
            .await
            .unwrap();
        }

        project(&db.conn, core_id).await.unwrap();

        let core = model::core_knowledge_entity::Entity::find_by_id(core_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (core.confidence.unwrap() - 0.95).abs() < 1e-6,
            "one chain must not be paid twice: {core:?}"
        );
    }

    /// A 429 is an instruction, not a fact: stop, cache nothing, and report the Retry-After.
    struct Limited;

    #[async_trait::async_trait]
    impl Engine for Limited {
        fn id(&self) -> &'static str {
            "limited"
        }

        fn plan(&self, core: &CoreView) -> Vec<Request> {
            vec![Request {
                provider: "limited",
                kind: "label".to_string(),
                shape: "v1".to_string(),
                query: core.label.clone(),
            }]
        }

        fn parse(&self, _core: &CoreView, _request: &Request, _raw: &Raw) -> Vec<Claim> {
            Vec::new()
        }

        async fn fetch(&self, _requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            Err(FetchError::RateLimited {
                retry_after_secs: 30,
            })
        }
    }

    /// The regression that mattered: a rate limit on the second batch must not discard the claims
    /// from the first. Fetching stops; parsing what we already have does not.
    struct Flaky {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Engine for Flaky {
        fn id(&self) -> &'static str {
            "flaky"
        }

        fn plan(&self, core: &CoreView) -> Vec<Request> {
            vec![Request {
                provider: "flaky",
                kind: "label".to_string(),
                shape: "v1".to_string(),
                query: core.label.clone(),
            }]
        }

        fn parse(&self, _core: &CoreView, request: &Request, raw: &Raw) -> Vec<Claim> {
            (raw.outcome == Outcome::Found)
                .then(|| Claim {
                    scheme: "wikidata".to_string(),
                    id: format!("Q{}", request.query.len()),
                    label: Some(request.query.clone()),
                    description: None,
                    classes: Vec::new(),
                    method: "exact_label".to_string(),
                    confidence: 0.9,
                })
                .into_iter()
                .collect()
        }

        async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) >= 1 {
                return Err(FetchError::RateLimited {
                    retry_after_secs: 60,
                });
            }
            Ok(requests
                .iter()
                .map(|request| {
                    (
                        request.query.clone(),
                        Raw {
                            outcome: Outcome::Found,
                            payload: Some(json!({"candidates": []})),
                        },
                    )
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn a_rate_limit_keeps_the_claims_already_fetched() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        for name in ["Alpha One", "Beta Two", "Gamma Three", "Delta Four"] {
            seed(&db, name).await;
        }

        let engines: Vec<Box<dyn Engine>> = vec![Box::new(Flaky {
            calls: Arc::new(AtomicUsize::new(0)),
        })];
        let report = resolve(&db, &engines, &Options::default()).await.expect("resolve");

        assert_eq!(report.rate_limited_after, Some(60), "{report:?}");
        assert_eq!(report.fetched, 2, "one successful batch, then the 429: {report:?}");
        assert_eq!(report.claims, BATCH, "claims from the first batch survive: {report:?}");
    }

    #[tokio::test]
    async fn a_rate_limit_stops_the_run_and_is_not_cached() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db, "Train Dreams").await;

        let engines: Vec<Box<dyn Engine>> = vec![Box::new(Limited)];
        let report = resolve(&db, &engines, &Options::default()).await.expect("resolve");

        assert_eq!(report.rate_limited_after, Some(30), "{report:?}");
        assert_eq!(report.fetched, 1, "{report:?}");
        assert_eq!(report.claims, 0, "{report:?}");
        assert_eq!(
            model::resolution_cache::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            0,
            "a rate limit must not be recorded as an answer"
        );
    }

    #[tokio::test]
    async fn dry_run_plans_without_fetching() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        seed(&db, "Train Dreams").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let engines = fake(HashMap::new(), calls.clone());
        let options = Options {
            dry_run: true,
            ..Default::default()
        };

        let report = resolve(&db, &engines, &options).await.expect("resolve");

        assert_eq!(report.planned, 1, "{report:?}");
        assert_eq!(report.fetched, 0, "{report:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
