//! Wikidata as an evidence provider.
//!
//! Four request shapes, all identity:
//!
//! | kind | how | batchable | why |
//! | --- | --- | --- | --- |
//! | `label` | SPARQL `VALUES` over the labels | yes | exact label → item, with descriptions and `P31` |
//! | `handle:P…` | SPARQL `VALUES` over the handles | yes | a platform account *is* an identity (`P2002`, `P2003`, …) |
//! | `title` | Wikipedia `pageprops` → `wikibase_item` | yes | a page title is unique where a label is not |
//! | `search` | MediaWiki full-text search (CirrusSearch) | no | fallback, only for entities the first stage missed |
//!
//! `plan` and `decision` are pure, so the whole decision can be tested from a fixture and re-run over
//! cached payloads for free. Only `fetch` touches the network, and nothing here calls it directly:
//! it goes through `Gateway::answers_for`, which keeps the cache, the pace and the budget.

use std::collections::{BTreeMap, HashSet};

use crate::fetch::{Answers, Gateway, Outcome, Request, Transport};
use crate::knowledge::evidence::Identity;
use crate::knowledge::identity::Resolver;
use crate::model;

use super::ranking::HANDLE_CONFIDENCE;
use super::transport::Wire;
use super::{WikidataCandidate, WikidataKind, answer, decision, plan, pool};

/// One authority: its decisions, and the wire it speaks through.
pub struct Wikidata {
    wire: Wire,
}

impl Wikidata {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self { wire: Wire::new()? })
    }
}

#[async_trait::async_trait]
impl Resolver for Wikidata {
    fn id(&self) -> &'static str {
        super::ID
    }

    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)> {
        ladder(&self.wire, entities, gateway).await
    }
}

/// One entity's questions: its id, and every `(step, request)` the planner built for it.
type Planned = (i32, Vec<(plan::Step, Request<WikidataKind>)>);

/// Ask every rung, then decide once.
///
/// The ladder used to decide per rung and stop at the first claim; this asks all of them and pools
/// the answers (see `pool`). Offline rebuilds are unaffected — `answers_for` serves the cache — and
/// the cost ordering is deliberately gone for now: the pool needs every answer, and a budget
/// question can only be answered after the quality question is.
async fn ladder<T: Transport<Kind = WikidataKind, Payload = Vec<WikidataCandidate>>>(
    transport: &T,
    entities: &[model::knowledge_entity::Model],
    gateway: &Gateway<'_>,
) -> Vec<(i32, Identity)> {
    let planned: Vec<Planned> = entities
        .iter()
        .map(|entity| (entity.id, plan::all_steps(entity)))
        .collect();

    // One request per question: `SurfaceTitle` re-plans the title request, so the set is deduped.
    let mut unique: Vec<Request<WikidataKind>> = Vec::new();
    let mut seen: HashSet<Request<WikidataKind>> = HashSet::new();
    for (_, steps) in &planned {
        for (_, request) in steps {
            if seen.insert(request.clone()) {
                unique.push(request.clone());
            }
        }
    }
    let answers = gateway.answers_for(transport, &unique).await;

    let mut identities = Vec::new();
    for entity in entities {
        let Some((_, steps)) = planned.iter().find(|(id, _)| *id == entity.id) else {
            continue;
        };
        // A property match *is* the identity, so nothing is pooled against it.
        if let Some(identity) = by_handle(steps, &answers) {
            identities.push((entity.id, identity));
            continue;
        }
        match pool::decide(entity, &gather(steps, &answers, Outcome::Found)) {
            Ok(identity) => identities.push((entity.id, identity)),
            Err(_) => {
                // A degraded answer may have held the claim we are discarding. Deciding the
                // degraded pool only to report it is the same rule as `decision::trusted`.
                if let Ok(identity) =
                    pool::decide(entity, &gather(steps, &answers, Outcome::Degraded))
                {
                    tracing::warn!(
                        entity = %entity.name,
                        qid = %identity.external_id,
                        method = identity.method.as_deref().unwrap_or(""),
                        "a degraded answer would have claimed; discarded"
                    );
                }
            }
        }
    }
    identities
}

/// The handle answer, when there is one: the item asserting this property value is the account
/// itself, and no other candidate can outrank a property match.
fn by_handle(
    steps: &[(plan::Step, Request<WikidataKind>)],
    answers: &Answers<WikidataKind, Vec<WikidataCandidate>>,
) -> Option<Identity> {
    let (_, request) = steps.iter().find(|(step, _)| *step == plan::Step::Handle)?;
    let raw = answers.get(request)?;
    if raw.outcome != Outcome::Found {
        return None;
    }
    let property = request.kind.property()?;
    let candidates = decision::referents(raw).ok()?;
    let first = candidates.first()?;
    Some(answer::identity(
        first,
        property.as_str(),
        HANDLE_CONFIDENCE,
    ))
}

/// One entity's pool from every answer of one outcome, deduplicated by QID.
///
/// `admit` still gates what is not a referent at all — a menu, a category, an impossible kind — but
/// no rung's *name* rule applies: which candidate is ours is the ranking's question now.
fn gather(
    steps: &[(plan::Step, Request<WikidataKind>)],
    answers: &Answers<WikidataKind, Vec<WikidataCandidate>>,
    outcome: Outcome,
) -> Vec<pool::Pooled> {
    let mut by_id: BTreeMap<String, pool::Pooled> = BTreeMap::new();
    for (step, request) in steps {
        // The handle is claimed whole, and `SurfaceTitle` asks the title's question again.
        if matches!(step, plan::Step::Handle | plan::Step::SurfaceTitle) {
            continue;
        }
        let Some(raw) = answers.get(request) else {
            continue;
        };
        if raw.outcome != outcome {
            continue;
        }
        let Ok(candidates) = decision::referents(raw) else {
            continue;
        };
        for (position, candidate) in candidates.into_iter().enumerate() {
            by_id
                .entry(candidate.id.clone())
                .or_insert_with(|| pool::Pooled {
                    candidate,
                    sources: Vec::new(),
                })
                .sources
                .push(pool::Source {
                    step: *step,
                    position,
                });
        }
    }
    by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Allowance, Answer, Outcome};
    use serde_json::{Value, json};

    fn found(candidates: Value) -> Answer<Vec<WikidataCandidate>> {
        Answer {
            outcome: Outcome::Found,
            body: Some(serde_json::from_value(candidates).expect("fixture candidates")),
        }
    }

    fn engine() -> Wikidata {
        Wikidata::new().expect("client should build")
    }

    /// The provider on every request is the resolver's id; a drift would split cache and provenance.
    #[test]
    fn every_planned_request_carries_the_authority_id() {
        let entity = entity("Train Dreams", Some("a 2011 novella"));
        for (_, request) in plan::all_steps(&entity) {
            assert_eq!(request.provider, crate::knowledge::identity::wikidata::ID);
        }
        assert_eq!(
            Resolver::id(&engine()),
            crate::knowledge::identity::wikidata::ID
        );
    }

    /// A wire that answers from a table and records what it was asked: no client, no database.
    /// The ladder is generic over the wire precisely so an ordering can be proven with this.
    #[derive(Default)]
    struct Canned {
        answers: std::collections::HashMap<(WikidataKind, String), Answer<Vec<WikidataCandidate>>>,
        seen: std::sync::Mutex<Vec<(WikidataKind, String)>>,
    }

    impl Canned {
        fn new(
            entries: impl IntoIterator<Item = (WikidataKind, String, Answer<Vec<WikidataCandidate>>)>,
        ) -> Self {
            let mut canned = Self::default();
            for (kind, query, answer) in entries {
                canned.answers.insert((kind, query), answer);
            }
            canned
        }

        fn asked(&self, kind: WikidataKind, query: &str) -> bool {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .any(|(asked, name)| *asked == kind && name == query)
        }
    }

    #[async_trait::async_trait]
    impl Transport for Canned {
        type Kind = WikidataKind;
        type Payload = Vec<WikidataCandidate>;

        fn batch_size(&self, _kind: &WikidataKind) -> usize {
            50
        }

        async fn fetch(
            &self,
            requests: &[Request<WikidataKind>],
        ) -> Result<Vec<(String, Answer<Vec<WikidataCandidate>>)>, crate::fetch::FetchError>
        {
            self.seen.lock().unwrap().extend(
                requests
                    .iter()
                    .map(|request| (request.kind, request.query.clone())),
            );
            Ok(requests
                .iter()
                .map(|request| {
                    let answer = self
                        .answers
                        .get(&(request.kind, request.query.clone()))
                        .cloned()
                        .unwrap_or_else(|| Answer::nothing(Outcome::Empty));
                    (request.query.clone(), answer)
                })
                .collect())
        }
    }

    fn entity(label: &str, description: Option<&str>) -> model::knowledge_entity::Model {
        let mut entity = crate::test_support::fixtures::stub_entity(1, label, "unknown");
        entity.description = description.map(str::to_string);
        entity
    }

    /// Every rung is asked, and the pool decides: here two answers offer the same surface, so the
    /// description separates them. The point of the fixture is that the *title* answer is no longer
    /// privileged by position — it wins on evidence.
    #[tokio::test]
    async fn the_pool_ranks_by_surface_then_description_and_asks_every_rung() {
        let seed = |kind: WikidataKind, query: &str, answer: Answer<Vec<WikidataCandidate>>| {
            (kind, query.to_string(), answer)
        };

        let title_answer = found(json!([
            {"id": "Q2", "label": "Train Dreams", "description": "novella by Denis Johnson"}
        ]));
        let label_answer = found(json!([
            {"id": "Q1", "label": "Train Dreams", "description": "something else entirely"}
        ]));
        let nothing = Answer::<Vec<WikidataCandidate>>::nothing(Outcome::Empty);
        let by_search = found(json!([
            {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"}
        ]));

        let fake = Canned::new([
            seed(WikidataKind::Title, "Train Dreams", title_answer),
            seed(WikidataKind::Label, "Train Dreams", label_answer),
            seed(WikidataKind::Title, "J.R.R. Tolkien", nothing.clone()),
            seed(WikidataKind::Label, "J.R.R. Tolkien", nothing.clone()),
            seed(WikidataKind::Search, "J.R.R. Tolkien", by_search),
        ]);

        let entities = vec![entity("Train Dreams", Some("novella by Denis Johnson")), {
            let mut second = entity("J.R.R. Tolkien", None);
            second.id = 2;
            second.description = Some("English author, philologist, and academic".to_string());
            second
        }];

        let allowance = Allowance::new(16, std::time::Duration::ZERO);
        let gateway = Gateway::new(None, Some(&allowance), true);
        let identities = ladder(&fake, &entities, &gateway).await;

        let ids: Vec<(i32, &str)> = identities
            .iter()
            .map(|(holder, identity)| (*holder, identity.external_id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q2"), (2, "Q892")], "{identities:?}");
        assert!(
            fake.asked(WikidataKind::Search, "Train Dreams"),
            "the pool needs every answer, so nothing is skipped after a claim"
        );
    }

    /// A surface tie is decided by the description: `Orthodoxy` the book corroborates, the concept
    /// does not. And when no rung can corroborate — the `Paris` shape — the exact surface still
    /// answers, because it is the only evidence there is.
    #[tokio::test]
    async fn the_pool_prefers_the_corroborated_candidate() {
        let seed = |kind: WikidataKind, query: &str, answer: Answer<Vec<WikidataCandidate>>| {
            (kind, query.to_string(), answer)
        };
        let nothing = Answer::<Vec<WikidataCandidate>>::nothing(Outcome::Empty);

        let concept = json!({
            "id": "Q100",
            "label": "Orthodoxy",
            "description": "adherence to the actual accepted belief, especially in religion",
            "classes": []
        });
        let fake = Canned::new([
            seed(
                WikidataKind::Title,
                "Orthodoxy",
                found(json!([concept.clone()])),
            ),
            seed(
                WikidataKind::Label,
                "Orthodoxy",
                found(json!([
                    concept,
                    {
                        "id": "Q200",
                        "label": "Orthodoxy",
                        "description": "Christian apologetics book by G. K. Chesterton",
                        "classes": []
                    }
                ])),
            ),
            seed(
                WikidataKind::Title,
                "Paris",
                found(json!([{
                    "id": "Q300",
                    "label": "Paris",
                    "description": "capital and most populous city in France",
                    "classes": []
                }])),
            ),
            seed(WikidataKind::Label, "Paris", nothing.clone()),
            seed(WikidataKind::Search, "Paris", nothing),
        ]);

        let entities = vec![
            entity(
                "Orthodoxy",
                Some("A 1908 philosophical and apologetic book by G. K. Chesterton"),
            ),
            {
                let mut second = entity("Paris", None);
                second.id = 2;
                second.entity_type = "place".to_string();
                second
            },
        ];

        let allowance = Allowance::new(16, std::time::Duration::ZERO);
        let gateway = Gateway::new(None, Some(&allowance), true);
        let identities = ladder(&fake, &entities, &gateway).await;

        let ids: Vec<(i32, &str)> = identities
            .iter()
            .map(|(holder, identity)| (*holder, identity.external_id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q200"), (2, "Q300")], "{identities:?}");
        assert!(
            fake.asked(WikidataKind::Search, "Orthodoxy"),
            "pooling asks every rung, including for the entity the label answered"
        );
    }
}
