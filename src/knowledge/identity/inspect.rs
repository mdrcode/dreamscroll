//! Walk one entity through the whole pipeline, in the order it happens.
//!
//! Recall, then ranking, then the resolver's ladder — each step printed with what the cache held
//! and what the decision was. Nothing here re-implements a stage: it calls the same blocker, the
//! same `Rank` and the same `decision` the engine calls, and reads `fetch_cache` with the same
//! expiry rule, and it applies the same standing vetoes. That is the point: if the inspector and
//! the run disagreed, the inspector would be lying.

use std::collections::{HashMap, HashSet};

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::fetch::cache::{FetchCache, State};
use crate::fetch::{Answer, Outcome, Request};
use crate::knowledge::blocker::SqlBlocker;
use crate::knowledge::identity::wikidata::{WikidataCandidate, WikidataKind, aggregate, plan};
use crate::knowledge::linker::heuristic::Heuristic;
use crate::knowledge::text;
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
    /// The ranked field: every candidate, best first, with the numbers that put it there.
    pub field: Vec<FieldRow>,
    /// The identity the field supports, when it supports one.
    pub decision: Option<FieldDecision>,
    /// Why it did not, when it did not — recorded by the resolver, not inferred here.
    pub refusal: Option<&'static str>,
    /// The decided identity is refused by a standing veto, so the engine would not write it.
    pub vetoed: bool,
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
    /// The first candidates the answer carried, for display: `(id, label, description)`.
    pub top: Vec<(String, String, String)>,
    /// Candidates the display does not show, so a truncated list is never read as the whole answer.
    pub hidden: usize,
}

/// One candidate in the field, with the score that placed it.
#[derive(Debug)]
pub struct FieldRow {
    pub qid: String,
    pub label: String,
    pub description: String,
    pub surface: f32,
    pub fit: f32,
    pub kind: Option<bool>,
    pub head: Option<bool>,
    pub year: i8,
    pub score: f32,
    pub chosen: bool,
}

/// What the field decided, when it decided.
#[derive(Debug)]
pub struct FieldDecision {
    pub qid: String,
    pub method: String,
    pub confidence: f32,
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

    let folded = text::comparable(&entity.name);
    let words = text::word_key(&folded);
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
    let scoring = Heuristic::scoring();
    let ranked = match &probe {
        Some(observation) => scoring
            .link(observation, &found)
            .into_iter()
            .map(|(index, score)| (found[index].id, found[index].name.clone(), score))
            .collect(),
        None => Vec::new(),
    };

    let vetoes = crate::knowledge::KnowledgeGraph::new(db).vetoed().await?;
    let walked = walk(db, &entity, &vetoes).await?;
    let identity = current_identity(db, entity.id).await?;

    Ok(Some(Report {
        entity,
        probe,
        candidates,
        ranked,
        steps: walked.steps,
        field: walked.field,
        decision: walked.decision,
        refusal: walked.refusal,
        vetoed: walked.vetoed,
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
    let wanted = text::comparable(target);
    if wanted.is_empty() {
        return Ok(None);
    }
    let live = model::knowledge_entity::Entity::find()
        .all(&db.conn)
        .await?;
    Ok(live
        .into_iter()
        .find(|row| row.merged_into_id.is_none() && text::comparable(&row.name) == wanted))
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

/// Everything a walk produced: what each question answered, and what the field made of it.
struct Walked {
    steps: Vec<Step>,
    field: Vec<FieldRow>,
    decision: Option<FieldDecision>,
    refusal: Option<&'static str>,
    vetoed: bool,
}

/// Ask every question, then decide once — the same two calls the resolver makes.
async fn walk(
    db: &DbHandle,
    entity: &model::knowledge_entity::Model,
    vetoes: &HashSet<(i32, String, String)>,
) -> anyhow::Result<Walked> {
    let cache = FetchCache::new(&db.conn);
    let planned = plan::all_steps(entity);
    let mut steps = Vec::with_capacity(planned.len());
    let mut answers: HashMap<Request<WikidataKind>, Answer<Vec<WikidataCandidate>>> =
        HashMap::new();

    for (_, request) in &planned {
        let lookup = cache
            .lookup::<_, Vec<WikidataCandidate>>(
                request.provider,
                &request.kind,
                &request.shape,
                &request.query,
            )
            .await?;
        let (top, hidden) = describe(lookup.body.as_deref().unwrap_or_default(), None);
        // An expired or unreadable row is a miss, exactly as it is to the resolver.
        if lookup.state == State::Fresh
            && !lookup.unreadable
            && let Some(outcome) = lookup.outcome
        {
            answers.insert(
                request.clone(),
                Answer {
                    outcome,
                    body: lookup.body,
                },
            );
        }
        steps.push(Step {
            kind: request.kind.to_string(),
            query: request.query.clone(),
            cache: lookup.state,
            outcome: lookup.outcome,
            top,
            hidden,
        });
    }

    let candidates = aggregate::gather(&planned, &answers, Outcome::Found);
    let (decision, refusal, vetoed) =
        if let Some(identity) = aggregate::by_handle(&planned, &answers) {
            let vetoed = vetoes.contains(&(
                entity.id,
                identity.scheme.clone(),
                identity.external_id.clone(),
            ));
            let decision = FieldDecision {
                qid: identity.external_id,
                method: identity.method.unwrap_or_default(),
                confidence: identity.confidence.unwrap_or_default(),
            };
            (Some(decision), None, vetoed)
        } else {
            match aggregate::decide(entity, &candidates) {
                Ok(identity) => {
                    let vetoed = vetoes.contains(&(
                        entity.id,
                        identity.scheme.clone(),
                        identity.external_id.clone(),
                    ));
                    let decision = FieldDecision {
                        qid: identity.external_id,
                        method: identity.method.unwrap_or_default(),
                        confidence: identity.confidence.unwrap_or_default(),
                    };
                    (Some(decision), None, vetoed)
                }
                Err(why) => (None, Some(why), false),
            }
        };

    let chosen = decision.as_ref().map(|decision| decision.qid.as_str());
    let field = aggregate::rank(entity, &candidates)
        .into_iter()
        .map(|ranked| {
            let candidate = &candidates[ranked.index].candidate;
            FieldRow {
                qid: candidate.id.clone(),
                label: candidate.label.clone().unwrap_or_default(),
                description: candidate.description.clone().unwrap_or_default(),
                surface: ranked.surface,
                fit: ranked.fit,
                kind: ranked.kind,
                head: ranked.head,
                year: ranked.year,
                score: aggregate::score(&ranked),
                chosen: chosen == Some(candidate.id.as_str()),
            }
        })
        .collect();

    Ok(Walked {
        steps,
        field,
        decision,
        refusal,
        vetoed,
    })
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
                    "hidden": step.hidden,
                }))
                .collect::<Vec<_>>(),
            "field": self
                .field
                .iter()
                .map(|row| serde_json::json!({
                    "qid": row.qid,
                    "label": row.label,
                    "description": row.description,
                    "surface": row.surface,
                    "fit": row.fit,
                    "kind": row.kind,
                    "head": row.head,
                    "year": row.year,
                    "score": row.score,
                    "chosen": row.chosen,
                }))
                .collect::<Vec<_>>(),
            "decision": self.decision.as_ref().map(|decision| serde_json::json!({
                "qid": decision.qid,
                "method": decision.method,
                "confidence": decision.confidence,
            })),
            "refusal": self.refusal,
            "vetoed": self.vetoed,
            "identity": self.identity.as_ref().map(|identity| serde_json::json!({
                "identity": identity.identity,
                "label": identity.label,
                "provider": identity.provider,
                "method": identity.method,
            })),
        })
    }
}

/// The candidates the inspection prints: the first four, plus the one the rung decided on when the
/// decision fell outside them — otherwise the report names a QID it never showed. `hidden` counts the
/// rest.
fn describe(
    candidates: &[WikidataCandidate],
    decided: Option<&str>,
) -> (Vec<(String, String, String)>, usize) {
    let row = |candidate: &WikidataCandidate| {
        (
            candidate.id.clone(),
            candidate.label.clone().unwrap_or_default(),
            candidate.description.clone().unwrap_or_default(),
        )
    };
    let mut rows: Vec<(String, String, String)> = candidates.iter().take(4).map(row).collect();
    if let Some(qid) = decided
        && !rows.iter().any(|(id, _, _)| id == qid)
        && let Some(candidate) = candidates.iter().find(|candidate| candidate.id == qid)
    {
        rows.push(row(candidate));
    }
    let hidden = candidates.len() - rows.len();
    (rows, hidden)
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
    use crate::knowledge::identity::wikidata::{WikidataCandidate, WikidataKind};
    use crate::test_support::{db_or_skip, fixtures};
    use sea_orm::EntityTrait;

    /// A veto is the engine's refusal, so the inspector must report it as one: the field did decide,
    /// and the graph would not write the identity.
    #[tokio::test]
    async fn a_standing_veto_is_not_reported_as_an_identity() {
        db_or_skip!(db);

        let id = fixtures::knowledge_entity_with_member(&db, "Inspected Thing").await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        // The title answer: an uncorroborated label is not eligible (a label is a name, not a
        // fact), while a title answer vouches for its candidate on surface alone.
        let request = plan::all_steps(&entity)
            .into_iter()
            .map(|(_, request)| request)
            .find(|request| request.kind == WikidataKind::Title)
            .expect("the ladder asks for a title");
        FetchCache::new(&db.conn)
            .put(
                "wikidata",
                &request.kind,
                &request.shape,
                &request.query,
                &Answer {
                    outcome: Outcome::Found,
                    body: Some(vec![WikidataCandidate {
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
        let decision = report.decision.as_ref().expect("the field decided");
        assert_eq!(decision.qid, "Q42");
        assert_eq!(decision.method, "wikipedia_title");
        assert!(report.vetoed);
        assert!(
            report.identity.is_none(),
            "the veto left no identity to hold"
        );
    }

    /// A decision can fall outside the four candidates the report prints — `M4 carbine` decided on a
    /// candidate the inspection never showed. The decided one is always rendered, and the count of
    /// the rest is stated so a truncated list is not read as the whole answer.
    #[test]
    fn the_decided_candidate_is_always_shown() {
        let candidate = |id: &str| WikidataCandidate {
            id: id.to_string(),
            label: Some(id.to_string()),
            description: None,
            classes: Vec::new(),
        };
        let five = [
            candidate("Q1"),
            candidate("Q2"),
            candidate("Q3"),
            candidate("Q4"),
            candidate("Q5"),
        ];

        let (rows, hidden) = describe(&five, None);
        assert_eq!(rows.len(), 4);
        assert_eq!(hidden, 1);

        let (rows, hidden) = describe(&five, Some("Q5"));
        assert_eq!(
            rows.iter()
                .map(|(id, _, _)| id.as_str())
                .collect::<Vec<_>>(),
            ["Q1", "Q2", "Q3", "Q4", "Q5"]
        );
        assert_eq!(hidden, 0, "everything shown is everything there was");

        let (rows, hidden) = describe(&five, Some("Q2"));
        assert_eq!(rows.len(), 4, "a shown decision adds no duplicate row");
        assert_eq!(hidden, 1);
    }
}
