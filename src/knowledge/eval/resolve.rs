//! Replay the resolver's ladder over the answer cache.
//!
//! The ladder is the driver's: `plan` first, then `plan_fallback` for whatever the first pass left
//! undecided. A harness that skips the fallback measures nothing — every claim the corpus earned
//! came from it, and an earlier version of this evaluator reported zero because it walked only the
//! first pass.
//!
//! Three ways a replay can lie: walking a ladder the driver does not walk; counting an uncached
//! entity as undecided; and stepping over an uncached rung to a claim a later one would never have
//! produced. The last is marked on the claim and counted by [`Report::partial`].

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::database::DbHandle;
use crate::fetch::{Answer, Outcome};
use crate::knowledge::evidence::Evidence;
use crate::knowledge::identity::wikidata::{Candidate, ID, parse, plan};
use crate::knowledge::KnowledgeGraph;

/// What one replay found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Entities considered.
    pub candidates: usize,
    /// Entities the replay would give a claim.
    pub claimed: usize,
    /// Considered, answered, and the claim refused: an editor vetoed that entity for that identity.
    pub vetoed: usize,
    /// Considered, answered, and no claim was warranted.
    pub undecided: usize,
    /// Considered, but at least one request had no fresh cached answer, so the decision could not be
    /// replayed. Reported apart from `undecided` because it is a gap in the evidence, not a verdict.
    pub uncached: usize,
    /// How many claims each method produced, by name.
    pub by_method: BTreeMap<String, usize>,
    /// The claims themselves, in id order — for `--verbose` and for reading a diff by eye.
    pub claims: Vec<Claimed>,
}

/// One claim the replay would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub entity_id: i32,
    pub name: String,
    /// The authority's identifier, e.g. `Q7844314`.
    pub identity: String,
    /// The ladder step that produced it: `exact_label`, `search_ranked`, `search_hint`, …
    pub method: String,
    /// At least one earlier rung had no fresh cached answer, so the real run would have asked it
    /// first. The claim is conditional on that answer refusing.
    pub partial: bool,
}

impl Report {
    /// Claims reached only by stepping over a rung whose answer was not cached.
    pub fn partial(&self) -> usize {
        self.claims.iter().filter(|claim| claim.partial).count()
    }
}

/// Replay every candidate's ladder against the cache.
///
/// `limit` caps how many candidates are considered, which is useful while a change is being
/// eyeballed; the order is by entity id, so a limit is stable between runs.
pub async fn replay(db: &DbHandle, limit: Option<usize>) -> anyhow::Result<Report> {
    // The engine's own selection, from the engine's own query: an evaluator that re-implements the
    // candidate rule measures itself. `wikidata` is the authority whose ladder this replays.
    let evidence = Evidence::from_handle(db);
    let entities = evidence
        .awaiting_authority(&[ID], limit, None, false)
        .await?;
    // A veto is a standing decision, so the claim it names is one the real run never writes.
    let vetoes = KnowledgeGraph::new(db).vetoed().await?;

    let mut report = Report::default();
    for view in &entities {
        report.candidates += 1;

        let mut claimed = None;
        let mut uncached = false;
        let mut stepped_over = false;
        let mut vetoed = false;
        for (rung, request) in plan::ladder(view) {
            let cached = db
                .conn
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT outcome, payload::text FROM fetch_cache \
                      WHERE provider = $1 AND kind = $2 AND shape = $3 AND query = $4 \
                        AND expires_at > now() LIMIT 1",
                    [
                        request.provider.into(),
                        request.kind.to_string().into(),
                        request.shape.as_str().into(),
                        request.query.as_str().into(),
                    ],
                ))
                .await?;
            let Some(row) = cached else {
                uncached = true;
                stepped_over = true;
                continue;
            };
            let outcome_text: String = row.try_get_by_index(0)?;
            let payload: Option<String> = row.try_get_by_index(1)?;
            let stored: Option<serde_json::Value> =
                payload.as_deref().and_then(|text| serde_json::from_str(text).ok());
            // Mirror `FetchCache::get_many`: an outcome this build cannot read, or an answered row
            // whose payload no longer decodes, is a miss — never an empty answer.
            let Some(outcome) = Outcome::parse(&outcome_text) else {
                uncached = true;
                stepped_over = true;
                continue;
            };
            let body: Option<Vec<Candidate>> = crate::fetch::cache::decode(stored.as_ref());
            if stored.is_some() && body.is_none() && outcome.answered() {
                uncached = true;
                stepped_over = true;
                continue;
            }
            let raw: Answer<Vec<Candidate>> = Answer {
                // The real outcome, so a degraded answer reads as degraded here too.
                outcome,
                body,
            };
            let claim = match rung {
                plan::Rung::SurfaceOnly => parse::admit(view, &raw).ok().and_then(|candidates| {
                    parse::surface_only(view, &candidates, &request)
                }),
                _ => parse::claims(view, &request, &raw).into_iter().next(),
            };
            if let Some(claim) = claim {
                if vetoes.contains(&(view.id, claim.scheme.clone(), claim.id.clone())) {
                    vetoed = true;
                } else {
                    claimed = Some((claim.id, claim.method, stepped_over));
                }
                break;
            }
        }

        match claimed {
            Some((identity, method, partial)) => {
                report.claimed += 1;
                *report.by_method.entry(method.clone()).or_default() += 1;
                report.claims.push(Claimed {
                    entity_id: view.id,
                    name: view.name.clone(),
                    identity,
                    method,
                    partial,
                });
            }
            None if vetoed => report.vetoed += 1,
            None if uncached => report.uncached += 1,
            None => report.undecided += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fetch::cache::FetchCache;
    use crate::knowledge::identity::wikidata::{Candidate, Kind, plan};
    use crate::knowledge::KnowledgeGraph;
    use crate::model;
    use crate::test_support::{db_or_skip, fixtures};
    use sea_orm::EntityTrait;

    /// Cache the answer that would make the label rung claim `qid`, and nothing else — so the title
    /// rung above it stays uncached and any claim found here is conditional on it.
    async fn cache_label(db: &DbHandle, id: i32, qid: &str) {
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
                        id: qid.to_string(),
                        label: Some(entity.name.clone()),
                        description: None,
                        classes: Vec::new(),
                    }]),
                },
            )
            .await
            .unwrap();
    }

    /// The three ways an entity can leave the replay: refused by a standing veto, undecided because
    /// its answers are not cached, and claimed by stepping over a rung that was not cached first.
    #[tokio::test]
    async fn replay_separates_vetoes_uncached_entities_and_partial_claims() {
        db_or_skip!(db);

        let vetoed = fixtures::knowledge_entity_with_member(&db, "Vetoed Thing").await;
        let partial = fixtures::knowledge_entity_with_member(&db, "Partial Thing").await;
        fixtures::knowledge_entity_with_member(&db, "Uncached Thing").await;

        cache_label(&db, vetoed, "Q1").await;
        cache_label(&db, partial, "Q2").await;
        KnowledgeGraph::new(&db)
            .veto(vetoed, "wikidata", "Q1", Some("test veto"))
            .await
            .unwrap();

        let report = replay(&db, None).await.unwrap();

        assert_eq!(report.candidates, 3);
        assert_eq!(report.claimed, 1, "{report:?}");
        assert_eq!(report.partial(), 1, "{report:?}");
        assert_eq!(report.vetoed, 1, "{report:?}");
        assert_eq!(report.uncached, 1, "{report:?}");
        assert_eq!(report.undecided, 0, "{report:?}");
        assert_eq!(report.claims[0].identity, "Q2");
        assert_eq!(report.claims[0].method, "exact_label");
        assert!(report.claims[0].partial);
    }
}
