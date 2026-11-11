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
//! `plan` and `parse` are pure, so the whole decision can be tested from a fixture and re-run over
//! cached payloads for free. Only `fetch` touches the network, and nothing here calls it directly:
//! it goes through `Gateway::resolve_requests`, which keeps the cache, the pace and the budget.

use std::collections::HashSet;

use super::transport::Wire;
use super::{Candidate, Kind, parse, plan};
use crate::fetch::{Answers, Gateway, Request, Transport};
use crate::knowledge::evidence::Claim;
use crate::knowledge::identity::Resolver;
use crate::model;

const USER_AGENT: &str = "dreamscroll-knowledge/0.1 (https://dreamscroll.ai)";

/// One authority: the policy, and the wire it speaks through.
pub struct Wikidata {
    wire: Wire,
}

impl Wikidata {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            wire: Wire::new(client),
        })
    }

    /// Turn answers into claims. First request to speak wins for an entity, which is why `plan`
    /// orders them: a handle is a stronger statement than a title, and a title than a label.
    fn collect(
        &self,
        entities: &[model::knowledge_entity::Model],
        plans: &[(usize, Request<Kind>)],
        answers: &Answers<Kind, Vec<Candidate>>,
        claims: &mut Vec<(i32, Claim)>,
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
            let parsed = parse::claims(entity, request, raw);
            if parsed.is_empty() {
                // Not wrong, just not usable here: a disambiguation page, a different kind of
                // thing, or a name with no item. The answer is still a fact about the provider, so
                // it keeps its full life in the cache — only a new `shape` may re-ask it.
                continue;
            }
            resolved.insert(entity.id);
            claims.extend(parsed.into_iter().map(|claim| (entity.id, claim)));
        }
    }
}

impl Wikidata {
    /// The ladder: handle, title and exact label in one pass; search for what is left; then the
    /// surface-only title. Generic over the wire, so a test can drive the ordering against a fake
    /// with no database and no client — [`Resolver::resolve`] supplies the real one.
    ///
    /// The engine sees only the claims that come back.
    async fn ladder<T: Transport<Kind = Kind, Payload = Vec<Candidate>>>(
        &self,
        transport: &T,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Claim)> {
        let mut claims = Vec::new();
        let mut resolved: HashSet<i32> = HashSet::new();

        // One step, one wave. The boundary lives here because no other authority knows that search
        // is the expensive route, and every wave must run offline too: `resolve_requests` decides
        // what to fetch, so it serves cached answers and silently fetches nothing — skipping a step
        // once threw those answers away, and an offline rebuild then reproduced less than the cache
        // held.
        for step in plan::Step::ORDER {
            let plans: Vec<(usize, Request<Kind>)> = entities
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
            let requests: Vec<Request<Kind>> =
                plans.iter().map(|(_, request)| request.clone()).collect();
            let answers = gateway.resolve_requests(transport, &requests).await;

            if step == plan::Step::SurfaceTitle {
                // The last resort reuses the title answer: it does not rank, it takes the article
                // bearing this exact name.
                for (index, request) in plans {
                    let entity = &entities[index];
                    if resolved.contains(&entity.id) {
                        continue;
                    }
                    let Some(raw) = answers.get(&request) else {
                        continue;
                    };
                    let Ok(candidates) = parse::admit(entity, raw) else {
                        continue;
                    };
                    if let Some(claim) = parse::surface_only(entity, &candidates, &request) {
                        resolved.insert(entity.id);
                        claims.push((entity.id, claim));
                    }
                }
            } else {
                self.collect(entities, &plans, &answers, &mut claims, &mut resolved);
            }
        }

        claims
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
    ) -> Vec<(i32, Claim)> {
        self.ladder(&self.wire, entities, gateway).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Answer, Outcome};
    use serde_json::{Value, json};

    use crate::knowledge::identity::wikidata::shape;

    fn request(kind: Kind, query: &str) -> Request<Kind> {
        Request {
            provider: "wikidata",
            shape: shape::of(&kind),
            kind,
            query: query.to_string(),
        }
    }

    /// A shape belongs to one kind: its own version prefix, carrying only the constants that kind's
    /// construction reads. Sharing a prefix made a change to one invalidate another, and appending a
    /// limit no kind sent made changing `SEARCH_LIMIT` throw away every label, title and handle
    /// answer for nothing — a cache miss that looks like a rate-limit problem.
    #[test]
    fn each_kind_owns_its_shape() {
        let label = shape::of(&Kind::Label);
        let title = shape::of(&Kind::Title);
        let search = shape::of(&Kind::Search);
        let handle = shape::of(&Kind::Handle("P2002"));

        assert!(search.contains("limit="), "{search}");
        for (kind, shape) in [("label", &label), ("title", &title), ("handle", &handle)] {
            assert!(!shape.contains("limit="), "{kind} sends no limit: {shape}");
        }
        assert!(
            !handle.contains("lang="),
            "a handle query reads no labels: {handle}"
        );
        assert_eq!(
            shape::of(&Kind::Handle("P2003")),
            handle,
            "another property is another kind, not another shape"
        );

        let prefixes: std::collections::BTreeSet<&str> = [&label, &title, &search, &handle]
            .iter()
            .filter_map(|shape| shape.split('|').next())
            .collect();
        assert_eq!(
            prefixes.len(),
            4,
            "each kind has to be bumpable on its own: {prefixes:?}"
        );
    }

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
        answers: std::collections::HashMap<(Kind, String), Answer<Vec<Candidate>>>,
        seen: std::sync::Mutex<Vec<(Kind, String)>>,
    }

    impl Canned {
        fn new(entries: impl IntoIterator<Item = (Kind, String, Answer<Vec<Candidate>>)>) -> Self {
            let mut canned = Self::default();
            for (kind, query, answer) in entries {
                canned.answers.insert((kind, query), answer);
            }
            canned
        }

        fn asked(&self, kind: Kind, query: &str) -> bool {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .any(|(asked, name)| *asked == kind && name == query)
        }
    }

    #[async_trait::async_trait]
    impl Transport for Canned {
        type Kind = Kind;
        type Payload = Vec<Candidate>;

        fn batch_size(&self, _kind: &Kind) -> usize {
            50
        }

        async fn fetch(
            &self,
            requests: &[Request<Kind>],
        ) -> Result<Vec<(String, Answer<Vec<Candidate>>)>, crate::fetch::FetchError> {
            self.seen
                .lock()
                .unwrap()
                .extend(requests.iter().map(|request| (request.kind, request.query.clone())));
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

    /// A Wikipedia title can redirect to a different subject: asking for `The Bounty Hunter Wars`
    /// lands on `Boba Fett`. "One title, one article" then reads as identity, so the answer has to be
    /// about our name before it is even considered.
    #[test]
    fn a_title_about_another_subject_is_refused() {
        let raw = found(json!([{
            "id": "Q51790",
            "label": "Boba Fett",
            "description": "fictional character from Star Wars",
        }]));
        let claims = parse::claims(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(Kind::Title, "The Bounty Hunter Wars"),
            &raw,
        );
        assert!(
            claims.is_empty(),
            "a redirect is not an identity: {claims:?}"
        );

        // …while a title whose label is our name still claims.
        let ok = found(json!([{
            "id": "Q16010557",
            "label": "The Bounty Hunter Wars",
            "description": "novel trilogy by K. W. Jeter",
        }]));
        let claims = parse::claims(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(Kind::Title, "The Bounty Hunter Wars"),
            &ok,
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].external_id, "Q16010557");

        // A title answer with no label is not a mismatch — the description still has to corroborate.
        let unlabelled = found(json!([{
            "id": "Q42574",
            "label": null,
            "description": "Canadian filmmaker and explorer (born 1954)",
        }]));
        let claims = parse::claims(
            &entity(
                "James Cameron",
                Some("Canadian filmmaker who served as executive producer on Point Break"),
            ),
            &request(Kind::Title, "James Cameron"),
            &unlabelled,
        );
        assert_eq!(claims.len(), 1, "a label-less answer can still be right");
        assert_eq!(claims[0].external_id, "Q42574");
    }

    #[test]
    fn an_exact_label_match_is_claimed() {
        let claims = parse::claims(
            &entity("Train Dreams", None),
            &request(Kind::Label, "Train Dreams"),
            &found(json!([{"id": "Q185971", "label": "Train Dreams", "description": "novella"}])),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].external_id, "Q185971");
        assert_eq!(claims[0].method, "exact_label");
        // A unique exact label is identity even with nothing to corroborate it — but it does not
        // reach the confidence a description-backed claim gets.
        assert!(
            (claims[0].confidence - 0.75).abs() < 1e-6,
            "{:?}",
            claims[0]
        );
    }

    /// A title answer always carries one candidate, so "only one" is not evidence there: the
    /// `Orthodoxy` article names the concept, and the book is a different page.
    #[test]
    fn a_title_cannot_claim_on_an_exact_label_alone() {
        let claims = parse::claims(
            &entity(
                "Orthodoxy",
                Some("A 1908 philosophical and apologetic book by G.K. Chesterton"),
            ),
            &request(Kind::Title, "Orthodoxy"),
            &found(json!([{
                "id": "Q383258",
                "label": "Orthodoxy",
                "description": "adherence to the actual accepted belief, especially in religion",
            }])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// On the label rung, one exact match *is* evidence: the query enumerated every holder of the
    /// string. With nothing to corroborate it — no description on either side — that is identity.
    #[test]
    fn a_lone_label_with_nothing_to_contradict_it_claims() {
        let claims = parse::claims(
            &entity("Tsoding", None),
            &request(Kind::Label, "Tsoding"),
            &found(json!([{"id": "Q1", "label": "Tsoding"}])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].method, "exact_label");
    }

    /// …and a description that shares no content word does not refute it: "British filmmaker" and
    /// "English film director" describe one person without agreeing on a single token.
    #[test]
    fn a_lone_label_claims_even_when_the_descriptions_share_no_word() {
        let claims = parse::claims(
            &entity(
                "Alfred Hitchcock",
                Some("English film director and producer"),
            ),
            &request(Kind::Label, "Alfred Hitchcock"),
            &found(json!([{
                "id": "Q7374",
                "label": "Alfred Hitchcock",
                "description": "British filmmaker (1899-1980)",
            }])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].method, "exact_label");
    }

    #[test]
    fn an_ambiguous_label_is_not_claimed() {
        // The Django case: two items share the surface, so the name cannot decide.
        let claims = parse::claims(
            &entity("Django", None),
            &request(Kind::Label, "Django"),
            &found(json!([
                {"id": "Q1", "label": "Django", "description": "web framework"},
                {"id": "Q2", "label": "Django", "description": "2021 video game"}
            ])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// Not the same surface: no claim, however close the spelling. The label stage does not rank.
    #[test]
    fn a_misspelt_label_is_not_claimed() {
        let claims = parse::claims(
            &entity("Sunset Boulevarde", None),
            &request(Kind::Label, "Sunset Boulevarde"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard"}])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    #[test]
    fn a_handle_match_is_stronger_than_a_label_match() {
        let claims = parse::claims(
            &entity("Tsoding", None),
            &request(Kind::Handle("P2002"), "tsoding"),
            &found(json!([{"id": "Q1234", "label": "Tsoding"}])),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].method, "P2002");
        assert_eq!(claims[0].confidence, 0.95);
    }

    /// The case the conservative rule used to throw away: eleven candidates share the label, and the
    /// capture's own description picks the 1950 film.
    #[test]
    fn a_description_picks_one_of_many_same_label_candidates() {
        let candidates = json!([
            {"id": "Q1", "label": "Sunset Boulevard", "description": "street in Los Angeles"},
            {"id": "Q2", "label": "Sunset Boulevard", "description": "1950 film noir directed by Billy Wilder"},
            {"id": "Q3", "label": "Sunset Boulevard", "description": "musical by Andrew Lloyd Webber"},
            {"id": "Q4", "label": "Sunset Boulevard", "description": "film directed by someone else"}
        ]);
        let claims = parse::claims(
            &entity(
                "Sunset Boulevard",
                Some("1950 American film noir directed by Billy Wilder about a faded silent-screen star"),
            ),
            &request(Kind::Label, "Sunset Boulevard"),
            &found(candidates),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].external_id, "Q2");
        assert_eq!(claims[0].method, "exact_label");
        assert!(claims[0].confidence >= 0.80, "{:?}", claims[0]);
    }

    /// A two-word description that shares one generic word must not win by being short.
    ///
    /// With today's conservative bar the correct candidate is rejected too (it scores 0.546 against
    /// 0.60, because Wikidata's description of George Washington shares only "first" and "president"
    /// with ours). That is a false negative we accept until the thresholds are calibrated against a
    /// labelled sample; the invariant worth locking now is that the painting never wins.
    #[test]
    fn a_short_generic_description_never_wins() {
        let claims = parse::claims(
            &entity(
                "George Washington",
                Some("An American military general and statesman who served as the first president of the United States"),
            ),
            &request(Kind::Label, "George Washington"),
            &found(json!([
                {"id": "Q79035760", "label": "George Washington", "description": "painting by American"},
                {"id": "Q23", "label": "George Washington", "description": "Founding Father and first U.S. president (1732–1799)"}
            ])),
        );
        assert!(
            claims.iter().all(|claim| claim.external_id != "Q79035760"),
            "a one-word match must not carry a claim: {claims:?}"
        );
    }

    /// With no candidate fitting the description, the illumination's hint decides.
    #[test]
    fn the_hint_decides_when_nothing_fits_the_description() {
        let mut view = entity("Stagecoach", Some("a western"));
        view.search_query = Some("Stagecoach 1939 film".to_string());
        let claims = parse::claims(
            &view,
            &request(Kind::Label, "Stagecoach"),
            &found(json!([
                {"id": "Q1", "label": "Stagecoach", "description": "1939 film directed by John Ford"},
                {"id": "Q2", "label": "Stagecoach", "description": "1986 film"},
                {"id": "Q3", "label": "Stagecoach", "description": "a horse-drawn vehicle"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].external_id, "Q1");
        assert_eq!(claims[0].method, "search_hint");
        assert!(claims[0].confidence <= 0.85, "hint claims are capped lower");
    }

    /// Two candidates fit our description equally: the hint must not break that tie. A hint that
    /// could pick one is still our own text echoing back, not independent evidence.
    #[test]
    fn the_hint_does_not_break_a_tie_between_qualifying_candidates() {
        let mut view = entity("Dune", Some("1965 science fiction novel by Frank Herbert"));
        view.search_query = Some("Dune novel Frank Herbert".to_string());
        let claims = parse::claims(
            &view,
            &request(Kind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"}
            ])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// No description to arbitrate: the tie is honest, so nothing is claimed.
    #[test]
    fn a_near_tie_claims_nothing() {
        let claims = parse::claims(
            &entity("Dune", None),
            &request(Kind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// …but with a description it resolves, which is the whole point of holding the text.
    #[test]
    fn a_description_breaks_the_dune_tie() {
        let claims = parse::claims(
            &entity("Dune", Some("1965 science fiction novel by Frank Herbert")),
            &request(Kind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].external_id, "Q1");
        assert_eq!(claims[0].method, "exact_label");
    }

    #[test]
    fn planning_prefers_the_handle_then_a_title_then_the_label() {
        let mut entity =
            crate::test_support::fixtures::stub_entity(1, "Alexey Kutepov", "real_person");
        entity.search_query = Some("Tsoding".to_string());
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("tsoding".to_string());
        let plans = plan::for_entity(&entity);
        assert_eq!(plans.len(), 3);
        assert_eq!(plans[0].kind, Kind::Handle("P2002"));
        // A title is unique where a label is not, so it is asked first and wins the claim.
        assert_eq!(plans[1].kind, Kind::Title);
        assert_eq!(plans[2].kind, Kind::Label);

        // Stage two searches the hint first — the disambiguating phrase — then the bare label, which
        // is the lookup blind spot (`J.R.R. Tolkien` vs `J. R. R. Tolkien`).
        let fallback = plan::fallback(&entity);
        assert_eq!(fallback.len(), 2, "{fallback:?}");
        assert_eq!(fallback[0].kind, Kind::Search);
        assert_eq!(fallback[0].query, "Tsoding");
        assert_eq!(fallback[1].query, "Alexey Kutepov");
    }

    /// A capture can name the platform and not the handle. Asking `VALUES { "" }` spends a call to
    /// cache a negative about nobody, so the rung is skipped and the name rungs still run.
    #[test]
    fn a_platform_without_a_handle_asks_nothing() {
        let mut entity =
            crate::test_support::fixtures::stub_entity(1, "Alexey Kutepov", "real_person");
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("   ".to_string());

        let plans = plan::for_entity(&entity);

        assert_eq!(plans.len(), 2, "{plans:?}");
        assert!(plans.iter().all(|request| request.kind != Kind::Handle("P2002")));
        assert_eq!(plans[0].kind, Kind::Title);
        assert_eq!(plans[1].kind, Kind::Label);
    }

    /// The hint is a *query*, not only a scoring signal: `Commission of Tears` alone finds meniscal
    /// tears, and the author in the hint is what would find the novel.
    #[test]
    fn the_hint_is_searched_before_the_bare_label() {
        let mut target = entity(
            "Commission of Tears",
            Some("A novel by António Lobo Antunes"),
        );
        target.search_query = Some("Commission of Tears Antonio Lobo Antunes".to_string());
        let plans = plan::fallback(&target);
        assert_eq!(plans.len(), 2, "{plans:?}");
        assert_eq!(plans[0].query, "Commission of Tears Antonio Lobo Antunes");
        assert_eq!(plans[1].query, "Commission of Tears");
    }

    #[test]
    fn a_platform_handle_uses_that_platforms_property() {
        let mut target = entity("Ada", None);
        target.platform = Some("threads".to_string());
        target.platform_handle = Some("@ada".to_string());

        let plans = plan::for_entity(&target);
        assert_eq!(plans[0].kind, Kind::Handle("P11892"), "{plans:?}");
        assert_eq!(plans[0].query, "@ada");
    }

    #[test]
    fn a_reddit_subreddit_uses_the_subreddit_property() {
        let mut target = entity("NFCWestMemeWar", None);
        target.platform = Some("reddit".to_string());
        target.platform_handle = Some("r/NFCWestMemeWar".to_string());

        let plans = plan::for_entity(&target);
        assert_eq!(plans[0].kind, Kind::Handle("P3984"), "{plans:?}");
        assert_eq!(plans[0].query, "NFCWestMemeWar");
    }

    #[test]
    fn a_reddit_user_uses_the_username_property() {
        let mut target = entity("spez", None);
        target.platform = Some("reddit".to_string());
        target.platform_handle = Some("u/spez".to_string());

        let plans = plan::for_entity(&target);
        assert_eq!(plans[0].kind, Kind::Handle("P4265"), "{plans:?}");
        assert_eq!(plans[0].query, "spez");
    }

    /// A hint that only repeats the name is not a second request.
    #[test]
    fn a_hint_equal_to_the_label_is_not_asked_twice() {
        let mut target = entity("Tsoding", None);
        target.search_query = Some("  TSODING ".to_string());
        let plans = plan::fallback(&target);
        assert_eq!(plans.len(), 1, "{plans:?}");
        assert_eq!(plans[0].query, "Tsoding");
    }

    /// Wikidata labels `J. R. R. Tolkien` with spaces; our label has none. Search finds it, and the
    /// surface comparison ignores the difference.
    #[test]
    fn a_search_answer_is_accepted_on_a_punctuation_variant() {
        let claims = parse::claims(
            &entity("J.R.R. Tolkien", Some("English author, philologist, and academic")),
            &request(Kind::Search, "J.R.R. Tolkien"),
            &found(json!([
                {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"},
                {"id": "Q3805090", "label": "J.R.R. Tolkien's The Lord of the Rings, Vol. I", "description": "DOS game"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].external_id, "Q892");
        assert_eq!(claims[0].method, "search_lookup");
    }

    /// A transliteration difference is a spelling of the same name, not a different name:
    /// `Dostoyevsky` is how Wikidata spells him, `Dostoevsky` is how the capture did.
    #[test]
    fn a_search_answer_with_a_misspelt_label_is_ranked() {
        let claims = parse::claims(
            &entity(
                "Fyodor Dostoevsky",
                Some("19th-century Russian novelist and philosopher renowned for psychological mastery"),
            ),
            &request(Kind::Search, "Fyodor Dostoevsky"),
            &found(json!([
                {"id": "Q991", "label": "Fyodor Dostoyevsky", "description": "Russian novelist and philosopher (1821–1881)"},
                {"id": "Q136199557", "label": "Fyodor Dostoevsky–Darkness and Dawn", "description": "book, 2024"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].external_id, "Q991");
    }

    /// The trap: an item whose *label* merely contains our name is a different referent, and its
    /// description overlaps ours precisely because it is about ours. A weak lookup must not become a
    /// weak claim — the search stage accepts a surface match or nothing.
    #[test]
    fn a_search_answer_that_only_mentions_our_name_is_refused() {
        for (name, description, label) in [
            (
                "Royal Navy",
                "the United Kingdom's naval warfare force",
                "Royal Navy fleet aircraft carrier fleet",
            ),
            (
                "Clawdbot",
                "an open-source personal AI agent harness",
                "Clawdbot Pro",
            ),
        ] {
            let claims = parse::claims(
                &entity(name, Some(description)),
                &request(Kind::Search, name),
                &found(json!([{"id": "Q1", "label": label, "description": "something else"}])),
            );
            assert!(claims.is_empty(), "{label}: {claims:?}");
        }
    }

    /// A Wikipedia title reaches an item whose Wikidata label is different: the item is
    /// `Symphony No. 7`, the article is `Symphony No. 7 (Sibelius)`.
    #[test]
    fn a_title_reaches_an_item_whose_label_differs() {
        let claims = parse::claims(
            &entity(
                "Symphony No. 7 (Sibelius)",
                Some("The final published symphony by Finnish composer Jean Sibelius"),
            ),
            &request(Kind::Title, "Symphony No. 7 (Sibelius)"),
            &found(json!([{
                "id": "Q261122",
                "label": "Symphony No. 7",
                "description": "symphony by Jean Sibelius",
                "classes": ["Q97363528"]
            }])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].external_id, "Q261122");
        assert_eq!(claims[0].method, "wikipedia_title");
    }

    /// A title that lands on a disambiguation page is a menu, not a referent.
    #[test]
    fn a_title_that_is_a_disambiguation_page_is_refused() {
        let claims = parse::claims(
            &entity("John Grant", Some("American singer-songwriter")),
            &request(Kind::Title, "John Grant"),
            &found(json!([{
                "id": "Q632837",
                "label": "John Grant",
                "description": "Wikimedia disambiguation page",
                "classes": ["Q4167410"]
            }])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// The bug this guard exists for: a *label* answer carrying a human-name disambiguation page was
    /// claimed, because the menu check lived in `parse_title` and the label rung never looked.
    #[test]
    fn a_label_answer_that_is_a_menu_is_refused() {
        let claims = parse::claims(
            &entity("Robert Chandler", Some("British literary translator")),
            &request(Kind::Label, "Robert Chandler"),
            &found(json!([{
                "id": "Q7342857",
                "label": "Robert Chandler",
                "description": "Wikimedia human name disambiguation page",
                "classes": ["Q22808320"]
            }])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// The subclass is not in every stored class list, so the description has to carry the rule too.
    #[test]
    fn a_menu_with_no_classes_is_refused_by_its_description() {
        let claims = parse::claims(
            &entity(
                "Rogue Heroes",
                Some("British historical drama television series"),
            ),
            &request(Kind::Label, "Rogue Heroes"),
            &found(json!([{
                "id": "Q115697563",
                "label": "Rogue Heroes",
                "description": "Wikimedia disambiguation page",
                "classes": []
            }])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    /// The ladder, driven end to end against a fake wire: `resolve` consults the title before the
    /// label, and only searches for the entity the first pass left unresolved.
    #[tokio::test]
    async fn resolve_prefers_a_title_and_searches_only_what_is_left() {
        use crate::fetch::Allowance;

        let seed = |kind: Kind, query: &str, answer: Answer<Vec<Candidate>>| {
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
            seed(Kind::Title, "Train Dreams", title_wins),
            seed(Kind::Label, "Train Dreams", label_loses),
            // The second entity misses both lookups and has to be searched for.
            seed(Kind::Title, "J.R.R. Tolkien", nothing.clone()),
            seed(Kind::Label, "J.R.R. Tolkien", nothing),
            seed(Kind::Search, "J.R.R. Tolkien", by_search),
        ]);

        let entities = vec![entity("Train Dreams", Some("novella by Denis Johnson")), {
            let mut second = entity("J.R.R. Tolkien", None);
            second.id = 2;
            second.description = Some("English author, philologist, and academic".to_string());
            second
        }];

        let allowance = Allowance::new(8, std::time::Duration::ZERO);
        let gateway = Gateway::new(None, Some(&allowance), true);
        let claims = engine().ladder(&fake, &entities, &gateway).await;

        let ids: Vec<(i32, &str)> = claims
            .iter()
            .map(|(id, claim)| (*id, claim.external_id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q2"), (2, "Q892")], "{claims:?}");
        assert!(
            fake.asked(Kind::Search, "J.R.R. Tolkien"),
            "the unresolved entity is searched for"
        );
        assert!(
            !fake.asked(Kind::Search, "Train Dreams"),
            "the resolved one is not searched"
        );
        assert!(
            !fake.asked(Kind::Label, "Train Dreams"),
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
        use crate::fetch::Allowance;

        let seed = |kind: Kind, query: &str, answer: Answer<Vec<Candidate>>| {
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
            seed(Kind::Title, "Orthodoxy", found(json!([concept.clone()]))),
            seed(
                Kind::Label,
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
                Kind::Title,
                "Paris",
                found(json!([{
                    "id": "Q300",
                    "label": "Paris",
                    "description": "capital and most populous city in France",
                    "classes": []
                }])),
            ),
            seed(Kind::Label, "Paris", nothing.clone()),
            seed(Kind::Search, "Paris", nothing),
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
        let claims = engine().ladder(&fake, &entities, &gateway).await;

        let ids: Vec<(i32, &str)> = claims
            .iter()
            .map(|(id, claim)| (*id, claim.external_id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q200"), (2, "Q300")], "{claims:?}");
        assert!(
            !fake.asked(Kind::Search, "Orthodoxy"),
            "the entity the label answered was never searched for"
        );
        assert!(
            fake.asked(Kind::Search, "Paris"),
            "the entity every rung refused fell through to search"
        );
    }
}
