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
//! `plan` and `interpret` are pure, so the whole decision can be tested from a fixture and re-run over
//! cached payloads for free. Only `fetch` touches the network, and nothing here calls it directly:
//! it goes through `Gateway::answers_for`, which keeps the cache, the pace and the budget.

use std::collections::HashSet;

use crate::fetch::{Answers, Gateway, Request, Transport};
use crate::knowledge::evidence::Identity;
use crate::knowledge::identity::Resolver;
use crate::model;

use super::transport::Wire;
use super::{Candidate, RequestKind, interpret, plan};

/// One authority: the policy, and the wire it speaks through.
pub struct Wikidata {
    wire: Wire,
}

impl Wikidata {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self { wire: Wire::new()? })
    }

    /// Turn one wave of answers into annotations. First request to speak wins for an entity, which
    /// is why `plan` orders them: a handle is a stronger statement than a title, than a label, than
    /// a search. An answer that supports no claim is not a failure — see the comment below.
    fn harvest(
        entities: &[model::knowledge_entity::Model],
        plans: &[(usize, Request<RequestKind>)],
        answers: &Answers<RequestKind, Vec<Candidate>>,
        identities: &mut Vec<(i32, Identity)>,
        resolved: &mut HashSet<i32>,
    ) {
        for (index, request) in plans {
            let entity = &entities[*index];
            if resolved.contains(&entity.id) {
                continue;
            }
            // The answer is keyed by the question itself, so the lookup borrows the request the
            // planner already built — no key to reassemble.
            let Some(raw) = answers.get(request) else {
                continue;
            };
            let Some(claim) = interpret::claim(entity, request, raw) else {
                // Not wrong, just not usable here: a disambiguation page, a different kind of
                // thing, or a name with no item. The answer is still a fact about the provider, so
                // it keeps its full life in the cache — only a new `shape` may re-ask it.
                continue;
            };
            resolved.insert(entity.id);
            identities.push((entity.id, claim));
        }
    }

    /// The last resort, and the one thing here that is not a question.
    ///
    /// [`plan::Step::SurfaceTitle`] plans the *identical* request as [`plan::Step::Title`], so this
    /// asks nothing new: the cached answer is simply decided a second way — no ranking, just the
    /// article that bears this exact name. That makes it a **decision wearing a step's clothes**,
    /// which is why it needs a branch here and a matching one in `inspect`, and why it is marked as
    /// a hack in `plan`. The shape it wants to become is a post-pass over the title answer once
    /// every real question has been asked (see R6 in `plan/wikidata-cleanup-20261112.md`).
    fn last_resort(
        entities: &[model::knowledge_entity::Model],
        plans: &[(usize, Request<RequestKind>)],
        answers: &Answers<RequestKind, Vec<Candidate>>,
        identities: &mut Vec<(i32, Identity)>,
        resolved: &mut HashSet<i32>,
    ) {
        for (index, request) in plans {
            let entity = &entities[*index];
            if resolved.contains(&entity.id) {
                continue;
            }
            let Some(raw) = answers.get(request) else {
                continue;
            };
            // `admit` runs again on an answer the title wave already gated. It is pure, so the
            // cost is a clone — the alternative is threading gated candidates between waves.
            let Ok(candidates) = interpret::admit(entity, raw) else {
                continue;
            };
            if let Some(claim) = interpret::surface_only(entity, &candidates, request) {
                resolved.insert(entity.id);
                identities.push((entity.id, claim));
            }
        }
    }

    /// The ladder: handle, title and exact label in one pass; search for what is left; then the
    /// surface-only title. Generic over the wire, so a test can drive the ordering against a fake
    /// with no database and no client — [`Resolver::resolve`] supplies the real one.
    ///
    /// The engine sees only the identities that come back.
    async fn ladder<T: Transport<Kind = RequestKind, Payload = Vec<Candidate>>>(
        transport: &T,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)> {
        let mut identities = Vec::new();
        let mut resolved: HashSet<i32> = HashSet::new();

        // One step, one wave. The boundary lives here because no other authority knows that search
        // is the expensive route, and every wave must run offline too: `answers_for` decides
        // what to fetch, so it serves cached answers and silently fetches nothing — skipping a step
        // once threw those answers away, and an offline rebuild then reproduced less than the cache
        // held.
        for step in plan::Step::ORDER {
            let plans: Vec<(usize, Request<RequestKind>)> = entities
                .iter()
                .enumerate()
                .filter(|(_, entity)| !resolved.contains(&entity.id))
                .flat_map(|(index, entity)| {
                    plan::step(step, entity)
                        .into_iter()
                        .map(move |request| (index, request))
                })
                .collect();
            if plans.is_empty() {
                continue;
            }
            let requests: Vec<Request<RequestKind>> =
                plans.iter().map(|(_, request)| request.clone()).collect();
            let answers = gateway.answers_for(transport, &requests).await;

            // `SurfaceTitle` is not a question — it re-decides the title answer — so it takes the
            // last-resort pass rather than the normal harvest. See `last_resort`.
            if step == plan::Step::SurfaceTitle {
                Self::last_resort(entities, &plans, &answers, &mut identities, &mut resolved);
            } else {
                Self::harvest(entities, &plans, &answers, &mut identities, &mut resolved);
            }
        }

        identities
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
        Self::ladder(&self.wire, entities, gateway).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Allowance, Answer, Outcome};
    use serde_json::{Value, json};

    fn found(candidates: Value) -> Answer<Vec<Candidate>> {
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
        for (_, request) in plan::ladder(&entity) {
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
        answers: std::collections::HashMap<(RequestKind, String), Answer<Vec<Candidate>>>,
        seen: std::sync::Mutex<Vec<(RequestKind, String)>>,
    }

    impl Canned {
        fn new(
            entries: impl IntoIterator<Item = (RequestKind, String, Answer<Vec<Candidate>>)>,
        ) -> Self {
            let mut canned = Self::default();
            for (kind, query, answer) in entries {
                canned.answers.insert((kind, query), answer);
            }
            canned
        }

        fn asked(&self, kind: RequestKind, query: &str) -> bool {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .any(|(asked, name)| *asked == kind && name == query)
        }
    }

    #[async_trait::async_trait]
    impl Transport for Canned {
        type Kind = RequestKind;
        type Payload = Vec<Candidate>;

        fn batch_size(&self, _kind: &RequestKind) -> usize {
            50
        }

        async fn fetch(
            &self,
            requests: &[Request<RequestKind>],
        ) -> Result<Vec<(String, Answer<Vec<Candidate>>)>, crate::fetch::FetchError> {
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

    /// The ladder, driven end to end against a fake wire: `resolve` consults the title before the
    /// label, and only searches for the entity the first pass left unresolved.
    #[tokio::test]
    async fn resolve_prefers_a_title_and_searches_only_what_is_left() {
        let seed = |kind: RequestKind, query: &str, answer: Answer<Vec<Candidate>>| {
            (kind, query.to_string(), answer)
        };

        let title_wins = found(json!([
            {"id": "Q2", "label": "Train Dreams (novella)", "description": "novella by Denis Johnson"}
        ]));
        let label_loses = found(json!([
            {"id": "Q1", "label": "Train Dreams", "description": "something else entirely"}
        ]));
        let nothing = Answer::<Vec<Candidate>>::nothing(Outcome::Empty);
        let by_search = found(json!([
            {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"}
        ]));

        // The title answers for the first entity, so its label must never be consulted — and a
        // different QID there makes the ordering visible.
        let fake = Canned::new([
            seed(RequestKind::Title, "Train Dreams", title_wins),
            seed(RequestKind::Label, "Train Dreams", label_loses),
            // The second entity misses both lookups and has to be searched for.
            seed(RequestKind::Title, "J.R.R. Tolkien", nothing.clone()),
            seed(RequestKind::Label, "J.R.R. Tolkien", nothing),
            seed(RequestKind::Search, "J.R.R. Tolkien", by_search),
        ]);

        let entities = vec![entity("Train Dreams", Some("novella by Denis Johnson")), {
            let mut second = entity("J.R.R. Tolkien", None);
            second.id = 2;
            second.description = Some("English author, philologist, and academic".to_string());
            second
        }];

        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(None, Some(&allowance), true);
        let claims = Wikidata::ladder(&fake, &entities, &gateway).await;

        let ids: Vec<(i32, &str)> = claims
            .iter()
            .map(|(holder, identity)| (*holder, identity.external_id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q2"), (2, "Q892")], "{claims:?}");
        assert!(
            fake.asked(RequestKind::Search, "J.R.R. Tolkien"),
            "the unresolved entity is searched for"
        );
        assert!(
            !fake.asked(RequestKind::Search, "Train Dreams"),
            "the resolved one is not searched"
        );
        assert!(
            !fake.asked(RequestKind::Label, "Train Dreams"),
            "the title resolved it, so the label wave never asks"
        );
        // Title × 2, label × 1 (Train Dreams dropped), search × 1 (Tolkien only).
        assert_eq!(
            fake.seen.lock().unwrap().len(),
            4,
            "the bundled pass would have asked five"
        );
    }

    /// The exact title is the last resort, not the first answer. `Orthodoxy` the article is the
    /// concept; the label rung, which carries the book, must get to speak first. And when no rung
    /// can corroborate anything — the `Paris` shape, no description, no hint — the title still
    /// answers, because a surface this exact is better than nothing at all.
    #[tokio::test]
    async fn a_surface_only_title_waits_for_every_other_rung() {
        let seed = |kind: RequestKind, query: &str, answer: Answer<Vec<Candidate>>| {
            (kind, query.to_string(), answer)
        };
        let nothing = Answer::<Vec<Candidate>>::nothing(Outcome::Empty);

        let concept = json!({
            "id": "Q100",
            "label": "Orthodoxy",
            "description": "adherence to the actual accepted belief, especially in religion",
            "classes": []
        });
        let fake = Canned::new([
            seed(
                RequestKind::Title,
                "Orthodoxy",
                found(json!([concept.clone()])),
            ),
            seed(
                RequestKind::Label,
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
            // `Paris`: the title is the only answer, and nothing contradicts it.
            seed(
                RequestKind::Title,
                "Paris",
                found(json!([{
                    "id": "Q300",
                    "label": "Paris",
                    "description": "capital and most populous city in France",
                    "classes": []
                }])),
            ),
            seed(RequestKind::Label, "Paris", nothing.clone()),
            seed(RequestKind::Search, "Paris", nothing),
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

        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(None, Some(&allowance), true);
        let claims = Wikidata::ladder(&fake, &entities, &gateway).await;

        let ids: Vec<(i32, &str)> = claims
            .iter()
            .map(|(holder, identity)| (*holder, identity.external_id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q200"), (2, "Q300")], "{claims:?}");
        assert!(
            !fake.asked(RequestKind::Search, "Orthodoxy"),
            "the entity the label answered was never searched for"
        );
        assert!(
            fake.asked(RequestKind::Search, "Paris"),
            "the entity every rung refused fell through to search"
        );
    }
}
