//! Walk one entity through the whole pipeline, in the order it happens.
//!
//! Recall, then ranking, then the resolver's ladder — each step printed with what the cache held
//! and what the decision was. Nothing here re-implements a stage: it calls the same blocker, the
//! same `Rank` and the same `parse` the engine calls, and reads `fetch_cache` with the same
//! expiry rule. That is the point: if the inspector and the run disagreed, the inspector would be
//! lying.

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

use crate::database::DbHandle;
use crate::fetch::{Answer, Outcome};
use crate::knowledge::linker::heuristic::Heuristic;
use crate::knowledge::identity::wikidata::{Candidate as Offered, parse, plan};
use crate::knowledge::normalize;
use crate::knowledge::scoring::Scoring;
use crate::knowledge::blocker::SqlBlocker;
use crate::model;

/// One entity, and everything the pipeline did with it.
#[derive(Debug)]
pub struct Report {
    pub entity: model::knowledge_entity::Model,
    /// The observation the blocker and the ranker were asked about — the entity's origin member.
    pub probe: Option<model::user_observed_entity::Model>,
    pub candidates: Vec<Candidate>,
    /// Candidates `Rank` would choose, best first, with the score it gave them.
    pub ranked: Vec<(i32, String, f32)>,
    pub steps: Vec<Step>,
    /// What the graph currently holds for this entity.
    pub claim: Option<Claim>,
}

/// One candidate, and which key generation found it.
#[derive(Debug)]
pub struct Candidate {
    pub id: i32,
    pub name: String,
    pub entity_type: String,
    /// The folded surface generation matched.
    pub by_surface: bool,
    /// The word-order generation matched.
    pub by_words: bool,
}

/// One rung of the resolver's ladder.
#[derive(Debug)]
pub struct Step {
    pub kind: String,
    pub query: String,
    pub cache: CacheState,
    /// What the stored answer said it was, when there is a row to read.
    pub outcome: Option<Outcome>,
    /// The first candidates the answer carried: `(id, label, description)`.
    pub top: Vec<(String, String, String)>,
    /// `(qid, method)` when this rung produced a claim.
    pub decision: Option<(String, String)>,
    /// Why it did not, when it did not — recorded by the resolver, not inferred here.
    pub refusal: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CacheState {
    /// A fresh answer was used.
    Fresh,
    /// Present but expired — the resolver treats it as a miss.
    Expired,
    /// Never asked.
    Missing,
}

#[derive(Debug)]
pub struct Claim {
    pub identity: String,
    pub label: Option<String>,
    pub asserted_by: String,
    pub method: Option<String>,
}

/// Inspect the entity that a name or id points at.
pub async fn inspect(db: &DbHandle, target: &str) -> anyhow::Result<Option<Report>> {
    let Some(entity) = find_entity(db, target).await? else {
        return Ok(None);
    };

    let probe = match entity.origin_observation_id {
        Some(id) => {
            model::user_observed_entity::Entity::find_by_id(id)
                .one(&db.conn)
                .await?
        }
        None => None,
    };

    let blocker = SqlBlocker::new(db);
    let found = match &probe {
        Some(observation) => {
            use crate::knowledge::blocker::Blocker;
            blocker.candidates(observation).await
        }
        None => blocker.candidates_for_name(&entity.name).await,
    };

    let folded = normalize::comparable(&entity.name);
    let words = normalize::word_key(&folded);
    let mut candidates = Vec::with_capacity(found.len());
    for row in &found {
        let keys = index_keys(db, row.id).await?;
        candidates.push(Candidate {
            id: row.id,
            name: row.name.clone(),
            entity_type: row.entity_type.clone(),
            by_surface: keys.0.contains(&folded),
            by_words: keys.1.contains(&words),
        });
    }

    // The same `Scoring` the linker asks, so "would it attach" here means what it means there.
    let scoring: Scoring = Heuristic::scoring();
    let ranked = match &probe {
        Some(observation) => scoring
            .link
            .rank(observation, &found)
            .into_iter()
            .map(|(index, score)| (found[index].id, found[index].name.clone(), score))
            .collect(),
        None => Vec::new(),
    };

    let steps = ladder(db, &entity).await?;
    let claim = current_claim(db, entity.id).await?;

    Ok(Some(Report {
        entity,
        probe,
        candidates,
        ranked,
        steps,
        claim,
    }))
}

/// The entity a target names: an id, or a surface that folds to the same key.
async fn find_entity(
    db: &DbHandle,
    target: &str,
) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
    if let Ok(id) = target.parse::<i32>()
        && let Some(row) = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await?
    {
        return Ok(Some(row));
    }
    let wanted = normalize::comparable(target);
    if wanted.is_empty() {
        return Ok(None);
    }
    let live = model::knowledge_entity::Entity::find()
        .all(&db.conn)
        .await?;
    Ok(live
        .into_iter()
        .find(|row| row.merged_into_id.is_none() && normalize::comparable(&row.name) == wanted))
}

/// `(surfaces, word_keys)` as stored for one entity.
async fn index_keys(db: &DbHandle, id: i32) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let row = db
        .conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT surfaces, word_keys FROM knowledge_blocking_index WHERE knowledge_entity_id = $1",
            [id.into()],
        ))
        .await?;
    let Some(row) = row else {
        return Ok((Vec::new(), Vec::new()));
    };
    // `varchar[]` comes back as `Vec<String>` through the same conversion the model uses.
    Ok((
        row.try_get_by_index::<Vec<String>>(0)?,
        row.try_get_by_index::<Vec<String>>(1)?,
    ))
}

/// Every rung the resolver would climb, in order, stopped at the first claim.
async fn ladder(
    db: &DbHandle,
    entity: &model::knowledge_entity::Model,
) -> anyhow::Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (rung, request) in plan::ladder(entity) {
        // The last resort asks the *same* title question again, so without a label the two steps
        // would look like a duplicate.
        let kind = match rung {
            plan::Rung::SurfaceOnly => format!("{} (surface)", request.kind),
            _ => request.kind.to_string(),
        };
        let row = db
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT outcome, payload::text, expires_at > now() FROM fetch_cache \
                  WHERE provider = $1 AND kind = $2 AND shape = $3 AND query = $4 \
                  ORDER BY id DESC LIMIT 1",
                [
                    request.provider.into(),
                    request.kind.to_string().into(),
                    request.shape.as_str().into(),
                    request.query.as_str().into(),
                ],
            ))
            .await?;

        let Some(row) = row else {
            steps.push(Step {
                kind: kind.clone(),
                query: request.query.clone(),
                cache: CacheState::Missing,
                outcome: None,
                top: Vec::new(),
                decision: None,
                refusal: "not asked",
            });
            continue;
        };

        let outcome_text: String = row.try_get_by_index(0)?;
        let payload: Option<String> = row.try_get_by_index(1)?;
        let fresh: bool = row.try_get_by_index(2)?;
        let stored: Option<serde_json::Value> =
            payload.as_deref().and_then(|text| serde_json::from_str(text).ok());
        let outcome = Outcome::parse(&outcome_text);
        let body: Option<Vec<Offered>> = crate::fetch::cache::decode(stored.as_ref());
        let top = describe(body.as_deref().unwrap_or_default());
        // What `FetchCache::get_many` makes of this row: an answered payload that no longer decodes
        // is a miss, so the gates must not run over an empty body and blame the answer.
        let unreadable =
            outcome.is_some_and(Outcome::answered) && stored.is_some() && body.is_none();

        // An expired row is a miss to the resolver, so it renders no decision here either.
        let mut refusal = if !fresh {
            "the cached answer is expired"
        } else if outcome.is_none() {
            "the stored outcome is not one this build knows"
        } else if unreadable {
            "the cached payload no longer decodes"
        } else {
            ""
        };
        let decision = match (fresh, outcome) {
            (true, Some(outcome)) if refusal.is_empty() => {
                let raw = Answer { outcome, body };
                let claim = match rung {
                    // The last resort does not rank: it takes the article bearing this exact name.
                    plan::Rung::SurfaceOnly => match parse::admit(entity, &raw) {
                        Ok(candidates) => parse::surface_only(entity, &candidates, &request),
                        Err(gate) => {
                            refusal = gate;
                            None
                        }
                    },
                    _ => parse::explained(entity, &request, &raw, &mut refusal)
                        .into_iter()
                        .next(),
                };
                claim.map(|claim| (claim.id, claim.method))
            }
            _ => None,
        };

        steps.push(Step {
            kind,
            query: request.query.clone(),
            cache: if fresh {
                CacheState::Fresh
            } else {
                CacheState::Expired
            },
            outcome,
            top,
            decision: decision.clone(),
            refusal,
        });
        if decision.is_some() {
            break;
        }
    }
    Ok(steps)
}

/// The first few candidates an answer carried, as the inspection prints them.
fn describe(candidates: &[Offered]) -> Vec<(String, String, String)> {
    candidates
        .iter()
        .take(4)
        .map(|candidate| {
            (
                candidate.id.clone(),
                candidate.label.clone().unwrap_or_default(),
                candidate.description.clone().unwrap_or_default(),
            )
        })
        .collect()
}

async fn current_claim(db: &DbHandle, id: i32) -> anyhow::Result<Option<Claim>> {
    let row = db
        .conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT i.external_id, i.label, i.asserted_by, i.method \
               FROM knowledge_claims c JOIN knowledge_identities i ON i.id = c.knowledge_identity_id \
              WHERE c.knowledge_entity_id = $1 AND c.retracted_at IS NULL LIMIT 1",
            [id.into()],
        ))
        .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(Claim {
        identity: row.try_get_by_index(0)?,
        label: row.try_get_by_index(1)?,
        asserted_by: row.try_get_by_index(2)?,
        method: row.try_get_by_index(3)?,
    }))
}
