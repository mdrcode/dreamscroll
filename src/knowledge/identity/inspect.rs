//! Walk one entity through the whole pipeline, in the order it happens.
//!
//! Recall, then ranking, then the resolver's ladder — each step printed with what the cache held
//! and what the decision was. Nothing here re-implements a stage: it calls the same blocker, the
//! same `Rank` and the same `parse` the engine calls, and reads `fetch_cache` with the same
//! expiry rule, and it applies the same standing vetoes. That is the point: if the inspector and
//! the run disagreed, the inspector would be lying.

use std::collections::HashSet;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::fetch::cache::{FetchCache, State};
use crate::fetch::{Answer, Outcome};
use crate::knowledge::blocker::SqlBlocker;
use crate::knowledge::identity::wikidata::{Candidate as Offered, parse, plan};
use crate::knowledge::linker::heuristic::Heuristic;
use crate::knowledge::normalize;
use crate::knowledge::scoring::Scoring;
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
    pub identity: Option<StoredIdentity>,
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
    pub cache: State,
    /// What the stored answer said it was, when there is a row to read.
    pub outcome: Option<Outcome>,
    /// The first candidates the answer carried: `(id, label, description)`.
    pub top: Vec<(String, String, String)>,
    /// `(qid, method)` when this rung produced an identity.
    pub decision: Option<(String, String)>,
    /// The rung's identity is refused by a standing veto, so the engine would not write it.
    pub vetoed: bool,
    /// Why it did not, when it did not — recorded by the resolver, not inferred here.
    pub refusal: &'static str,
}

#[derive(Debug)]
pub struct StoredIdentity {
    pub identity: String,
    pub label: Option<String>,
    pub provider: String,
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

    let vetoes = crate::knowledge::KnowledgeGraph::new(db).vetoed().await?;
    let steps = ladder(db, &entity, &vetoes).await?;
    let identity = current_identity(db, entity.id).await?;

    Ok(Some(Report {
        entity,
        probe,
        candidates,
        ranked,
        steps,
        identity,
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
    let row = model::knowledge_blocking_index::Entity::find_by_id(id)
        .one(&db.conn)
        .await?;
    Ok(row
        .map(|row| (row.surfaces, row.word_keys))
        .unwrap_or_default())
}

/// Every rung the resolver would climb, in order, stopped at the first identity.
async fn ladder(
    db: &DbHandle,
    entity: &model::knowledge_entity::Model,
    vetoes: &HashSet<(i32, String, String)>,
) -> anyhow::Result<Vec<Step>> {
    let mut steps = Vec::new();
    let cache = FetchCache::new(&db.conn);
    for (rung, request) in plan::ladder(entity) {
        // The last resort asks the *same* title question again, so without a label the two steps
        // would look like a duplicate.
        let kind = match rung {
            plan::Step::SurfaceTitle => format!("{} (surface)", request.kind),
            _ => request.kind.to_string(),
        };
        let lookup = cache
            .lookup::<_, Vec<Offered>>(
                request.provider,
                &request.kind,
                &request.shape,
                &request.query,
            )
            .await?;
        let top = describe(lookup.body.as_deref().unwrap_or_default());

        // An expired row is a miss to the resolver, so it renders no decision here either; an
        // unreadable answered payload is the miss `get_many` makes of it.
        let mut refusal = match (lookup.state, lookup.unreadable) {
            (State::Missing, _) => "not asked",
            (State::Expired, _) => "the cached answer is expired",
            (State::Fresh, true) => "the cached payload no longer decodes",
            _ => "",
        };
        let mut vetoed = false;
        let decision = if lookup.state == State::Fresh && refusal.is_empty() {
            let raw = Answer {
                outcome: lookup.outcome.expect("a present row has an outcome"),
                body: lookup.body,
            };
            let identity = match rung {
                // The last resort does not rank: it takes the article bearing this exact name.
                plan::Step::SurfaceTitle => match parse::admit(entity, &raw) {
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
            identity.map(|identity| {
                vetoed = vetoes.contains(&(
                    entity.id,
                    identity.scheme.clone(),
                    identity.external_id.clone(),
                ));
                (identity.external_id, identity.method)
            })
        } else {
            None
        };

        steps.push(Step {
            kind,
            query: request.query.clone(),
            cache: lookup.state,
            outcome: lookup.outcome,
            top,
            decision: decision.clone(),
            vetoed,
            refusal,
        });
        if decision.is_some() {
            break;
        }
    }
    Ok(steps)
}

impl Report {
    pub fn to_json(&self) -> serde_json::Value {
        let entity = &self.entity;
        serde_json::json!({
            "entity": {
                "id": entity.id,
                "name": entity.name,
                "alt_names": entity.alt_names,
                "entity_type": entity.entity_type,
                "alt_types": entity.alt_types,
                "description": entity.description,
                "alt_descriptions": entity.alt_descriptions,
                "search_query": entity.search_query,
                "alt_search_queries": entity.alt_search_queries,
                "platform": entity.platform,
                "platform_handle": entity.platform_handle,
                "platform_display_name": entity.platform_display_name,
                "status": entity.status.as_str(),
                "confidence": entity.confidence,
                "merged_into_id": entity.merged_into_id,
                "origin_observation_id": entity.origin_observation_id,
            },
            "probe": self.probe.as_ref().map(|probe| serde_json::json!({
                "id": probe.id,
                "capture_id": probe.capture_id,
                "illumination_id": probe.illumination_id,
                "position": probe.position,
                "name": probe.name,
                "entity_type": probe.entity_type,
                "description": probe.description,
                "search_query": probe.search_query,
                "platform": probe.platform,
                "platform_handle": probe.platform_handle,
                "platform_display_name": probe.platform_display_name,
            })),
            "candidates": self
                .candidates
                .iter()
                .map(|candidate| serde_json::json!({
                    "id": candidate.id,
                    "name": candidate.name,
                    "entity_type": candidate.entity_type,
                    "by_surface": candidate.by_surface,
                    "by_words": candidate.by_words,
                }))
                .collect::<Vec<_>>(),
            "ranked": self
                .ranked
                .iter()
                .map(|(id, name, score)| serde_json::json!({
                    "id": id,
                    "name": name,
                    "score": score,
                }))
                .collect::<Vec<_>>(),
            "steps": self
                .steps
                .iter()
                .map(|step| serde_json::json!({
                    "kind": step.kind,
                    "query": step.query,
                    "cache": step.cache.as_str(),
                    "outcome": step.outcome.map(|outcome| outcome.as_str()),
                    "top": step
                        .top
                        .iter()
                        .map(|(id, label, description)| serde_json::json!({
                            "id": id,
                            "label": label,
                            "description": description,
                        }))
                        .collect::<Vec<_>>(),
                    "decision": step.decision.as_ref().map(|(qid, method)| serde_json::json!({
                        "qid": qid,
                        "method": method,
                    })),
                    "vetoed": step.vetoed,
                    "refusal": step.refusal,
                }))
                .collect::<Vec<_>>(),
            "identity": self.identity.as_ref().map(|identity| serde_json::json!({
                "identity": identity.identity,
                "label": identity.label,
                "provider": identity.provider,
                "method": identity.method,
            })),
        })
    }
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

async fn current_identity(db: &DbHandle, id: i32) -> anyhow::Result<Option<StoredIdentity>> {
    let identity = model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(id))
        .filter(model::knowledge_identity::Column::Scheme.eq("wikidata"))
        .order_by_asc(model::knowledge_identity::Column::Id)
        .one(&db.conn)
        .await?;
    Ok(identity.map(|identity| StoredIdentity {
        identity: identity.external_id,
        label: identity.label,
        provider: identity.provider,
        method: identity.method,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fetch::cache::FetchCache;
    use crate::fetch::{Answer, Outcome};
    use crate::knowledge::KnowledgeGraph;
    use crate::knowledge::identity::wikidata::{Candidate, Kind};
    use crate::test_support::{db_or_skip, fixtures};
    use sea_orm::EntityTrait;

    /// A veto is the engine's refusal, so the inspector must report it as one: the rung did produce
    /// an identity, and the graph would not write it.
    #[tokio::test]
    async fn a_standing_veto_is_not_reported_as_an_identity() {
        db_or_skip!(db);

        let id = fixtures::knowledge_entity_with_member(&db, "Inspected Thing").await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let request = plan::ladder(&entity)
            .into_iter()
            .map(|(_, request)| request)
            .find(|request| request.kind == Kind::Label)
            .expect("the ladder asks for a label");
        FetchCache::new(&db.conn)
            .put(
                "wikidata",
                &request.kind,
                &request.shape,
                &request.query,
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(vec![Candidate {
                        id: "Q42".to_string(),
                        label: Some(entity.name.clone()),
                        description: None,
                        classes: Vec::new(),
                    }]),
                },
            )
            .await
            .unwrap();
        KnowledgeGraph::new(&db)
            .veto(id, "wikidata", "Q42", Some("test veto"))
            .await
            .unwrap();

        let report = inspect(&db, &id.to_string())
            .await
            .unwrap()
            .expect("entity");
        let label = report
            .steps
            .iter()
            .find(|step| step.kind == "label")
            .expect("label rung");
        assert_eq!(
            label.decision,
            Some(("Q42".to_string(), "exact_label".to_string()))
        );
        assert!(label.vetoed);
        assert!(
            report.identity.is_none(),
            "the veto left no identity to hold"
        );
    }
}
