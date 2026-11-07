//! Replay the resolver's ladder over the answer cache.
//!
//! The ladder is the driver's: `plan` first, then `plan_fallback` for whatever the first pass left
//! undecided. A harness that skips the fallback measures nothing — every claim the corpus earned
//! came from it, and an earlier version of this evaluator reported zero because it walked only the
//! first pass.

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

use crate::database::DbHandle;
use crate::knowledge::identity::Raw;
use crate::knowledge::identity::cache::Outcome;
use crate::knowledge::identity::wikidata::Wikidata;
use crate::model;

/// Entities `identity resolve` would consider: live, and with no ontology claim. A platform handle
/// alone still deserves a QID, so it does not exclude an entity.
const CANDIDATES: &str = "
SELECT e.id FROM knowledge_entities e
 WHERE e.merged_into_id IS NULL
   AND NOT EXISTS (
       SELECT 1 FROM knowledge_claims c
         JOIN knowledge_identities i ON i.id = c.knowledge_identity_id
        WHERE c.knowledge_entity_id = e.id AND c.retracted_at IS NULL
          AND i.source <> 'platform')
 ORDER BY e.id";

/// What one replay found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Entities considered.
    pub candidates: usize,
    /// Entities the replay would give a claim.
    pub claimed: usize,
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
}

/// Replay every candidate's ladder against the cache.
///
/// `limit` caps how many candidates are considered, which is useful while a change is being
/// eyeballed; the order is by entity id, so a limit is stable between runs.
pub async fn replay(db: &DbHandle, limit: Option<usize>) -> anyhow::Result<Report> {
    let engine = Wikidata::new()?;
    let ids: Vec<i32> = db
        .conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            CANDIDATES,
        ))
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<i32>(0))
        .collect::<Result<_, _>>()?;

    let mut report = Report::default();
    for id in ids.into_iter().take(limit.unwrap_or(usize::MAX)) {
        report.candidates += 1;
        let Some(view) = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await?
        else {
            continue;
        };

        let mut ladder = engine.plan(&view);
        ladder.extend(engine.plan_fallback(&view));

        let mut claimed = None;
        let mut uncached = false;
        for request in ladder {
            let cached = db
                .conn
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT outcome, payload::text FROM resolution_cache \
                      WHERE provider = $1 AND kind = $2 AND shape = $3 AND query = $4 \
                        AND expires_at > now() LIMIT 1",
                    [
                        request.provider.into(),
                        request.kind.to_key().into(),
                        request.shape.as_str().into(),
                        request.query.as_str().into(),
                    ],
                ))
                .await?;
            let Some(row) = cached else {
                uncached = true;
                continue;
            };
            let outcome: String = row.try_get_by_index(0)?;
            let payload: Option<String> = row.try_get_by_index(1)?;
            let raw = Raw {
                outcome: if outcome == "found" {
                    Outcome::Found
                } else {
                    Outcome::Empty
                },
                payload: payload.as_deref().map(serde_json::from_str).transpose()?,
            };
            if let Some(claim) = engine.parse(&view, &request, &raw).into_iter().next() {
                claimed = Some((claim.id, claim.method));
                break;
            }
        }

        match claimed {
            Some((identity, method)) => {
                report.claimed += 1;
                *report.by_method.entry(method.clone()).or_default() += 1;
                report.claims.push(Claimed {
                    entity_id: view.id,
                    name: view.name.clone(),
                    identity,
                    method,
                });
            }
            None if uncached => report.uncached += 1,
            None => report.undecided += 1,
        }
    }
    Ok(report)
}
