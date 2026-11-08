//! Wikidata as an evidence provider.
//!
//! Three request shapes, all identity:
//!
//! | kind | how | batchable | why |
//! | --- | --- | --- | --- |
//! | `label` | SPARQL `VALUES` over the labels | yes | exact label → item, with descriptions and `P31` |
//! | `handle:P…` | SPARQL `VALUES` over the handles | yes | a platform account *is* an identity (`P2002`, `P2003`, …) |
//! | `search` | `wbsearchentities` | no | full-text fallback, only for entities the first stage missed |
//!
//! `plan` and `parse` are pure, so the whole decision can be tested from a fixture and re-run over
//! cached payloads for free. Only `fetch` touches the network, and nothing here calls it directly:
//! it goes through `Gateway::resolve_requests`, which keeps the cache, the pace and the budget.

use std::collections::HashSet;

use super::answer;
use super::policy;
use super::types;
use crate::knowledge::identity::{
    Answers, BATCH, Candidate, Claim, FetchError, Gateway, Kind, Raw, Request, Resolver, Transport,
};
use crate::knowledge::normalize;
use crate::model;

use super::ranking::HANDLE_CONFIDENCE;

pub(super) const SEARCH_ENDPOINT: &str = "https://www.wikidata.org/w/api.php";
/// Article-title reconciliation. A Wikipedia title is unique where a Wikidata label is not: 213 items
/// are labelled `Paris`, but only one page is titled `Paris`.
pub(super) const WIKIPEDIA_ENDPOINT: &str = "https://en.wikipedia.org/w/api.php";
const USER_AGENT: &str = "dreamscroll-knowledge/0.1 (https://dreamscroll.ai)";
pub(super) const LANGUAGE: &str = "en";

/// A platform account is an identity, and each platform is a different property. Adding one is a row
/// in this table, not a new provider.
///
/// Every property here was verified against Wikidata on 2026-10-31. Unmapped platforms (`threads`,
/// `reddit`, `other`) are deliberate: a wrong property would produce confident, fabricated
/// identities, which is worse than leaving the entity unresolved.
fn handle_property(platform: &str) -> Option<&'static str> {
    match platform.to_ascii_lowercase().as_str() {
        "x" | "twitter" | "x_twitter" => Some("P2002"),
        "instagram" => Some("P2003"),
        "tiktok" => Some("P7085"),
        "youtube" => Some("P2397"),
        "github" => Some("P2037"),
        "mastodon" => Some("P4033"),
        _ => None,
    }
}

pub struct Wikidata {
    pub(super) client: reqwest::Client,
}

impl Wikidata {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self { client })
    }

    /// Part of the cache key: how the request is built. Change a parameter and the key changes, so a
    /// stale answer cannot be served against a different question.
    ///
    /// Readable on purpose. It was a blake3 hash truncated to twelve characters, which has the same
    /// property — but a row in `resolution_cache` should say which question it answered, and a hash
    /// says nothing when you are looking at a stale answer.
    fn shape(kind: &Kind) -> String {
        // Per kind, so a transport change invalidates exactly the answers it changes. `search` moved
        // from `wbsearchentities` to MediaWiki full text, hence its own version.
        // v3 for `label`: the query now asks for `P31` alongside the label, so an answer fetched
        // before that is an answer to a different question.
        let version = match kind {
            Kind::Search => "wikidata/v3-search",
            Kind::Label => "wikidata/v3-label",
            _ => "wikidata/v2",
        };
        format!(
            "{version}/{}|lang={LANGUAGE}|limit={}",
            kind.to_key(),
            super::SEARCH_LIMIT
        )
    }

    /// The first pass: a handle, then a title, then an exact label. Pure, so the order is testable
    /// without a network — and public for the same reason: `examples/eval_resolve` re-runs the whole
    /// decision over cached payloads.
    pub fn plan(&self, entity: &model::knowledge_entity::Model) -> Vec<Request> {
        let mut requests = Vec::new();
        if let (Some(platform), Some(handle)) = (&entity.platform, &entity.platform_handle)
            && let Some(property) = handle_property(platform)
        {
            let kind = Kind::Handle(property);
            requests.push(Request {
                provider: "wikidata",
                shape: Self::shape(&kind),
                kind,
                query: handle.trim().to_string(),
            });
        }
        let label = entity.name.trim();
        if !label.is_empty() {
            // Title before label: a capture's name is often the *article* title
            // (`Symphony No. 7 (Sibelius)`), and the item is labelled `Symphony No. 7`. The label
            // lookup cannot see that; the title can.
            requests.push(Request {
                provider: "wikidata",
                shape: Self::shape(&Kind::Title),
                kind: Kind::Title,
                query: label.to_string(),
            });
            requests.push(Request {
                provider: "wikidata",
                shape: Self::shape(&Kind::Label),
                kind: Kind::Label,
                query: label.to_string(),
            });
        }
        requests
    }

    /// The second pass: search for the **hint**, then the **bare label**, for entities the first pass
    /// left unresolved.
    ///
    /// The hint goes first because it is the phrase that disambiguates, and until now it was only a
    /// *scoring* signal — consulted against candidates some other question had already produced. It is
    /// a lookup, though, and `wbsearchentities` matches labels and aliases rather than text, so a long
    /// hint (`Commission of Tears Antonio Lobo Antunes`) usually returns nothing. That is a limit of
    /// the endpoint, not a reason to withhold the question: over-fetching is the resolver's job, and
    /// refusing a wrong candidate is the decider's.
    ///
    /// The bare label stays because it is a lookup fallback, not a ranking signal. Wikidata labels
    /// `J. R. R. Tolkien` and `D. H. Lawrence` *with spaces*; our exact-label SPARQL sends
    /// `J.R.R. Tolkien` and gets nothing, while search finds Q892 immediately. Measured on 20 random
    /// empty-label entities: 4 recoverable this way.
    ///
    /// The pass boundary lives here, not in the engine: no other authority knows that search is the
    /// expensive route.
    pub fn plan_fallback(&self, entity: &model::knowledge_entity::Model) -> Vec<Request> {
        let label = entity.name.trim();
        if label.is_empty() {
            return Vec::new();
        }
        let mut requests = Vec::new();
        if let Some(hint) = entity.search_query.as_deref() {
            let hint = hint.trim();
            // A hint that only repeats the name is not a second request.
            if !hint.is_empty() && normalize::comparable(hint) != normalize::comparable(label) {
                requests.push(Request {
                    provider: "wikidata",
                    shape: Self::shape(&Kind::Search),
                    kind: Kind::Search,
                    query: hint.to_string(),
                });
            }
        }
        requests.push(Request {
            provider: "wikidata",
            shape: Self::shape(&Kind::Search),
            kind: Kind::Search,
            query: label.to_string(),
        });
        requests
    }

    /// Turn an answer into claims. Pure: same entity plus same answer, same claims.
    pub fn parse(
        &self,
        entity: &model::knowledge_entity::Model,
        request: &Request,
        raw: &Raw,
    ) -> Vec<Claim> {
        let mut reason = "";
        self.parse_explained(entity, request, raw, &mut reason)
    }

    /// The gates every answer passes before a rung decides: a menu is not a referent, and the right
    /// name can still be the wrong kind of thing. Only recognised classes reject — an unknown one
    /// passes (see `types`), so the table being incomplete cannot lose a candidate.
    ///
    /// Split out because the last-resort pass needs the same gates without a rung's decision.
    fn admit(
        &self,
        entity: &model::knowledge_entity::Model,
        raw: &Raw,
    ) -> Result<Vec<Candidate>, &'static str> {
        if !raw.outcome.answered() {
            return Err("the answer carried no candidate");
        }
        let mut candidates = raw.candidates.clone();
        // A menu is not a referent, on *every* rung. This used to be checked in `parse_title` only,
        // and the label rung — which is where all 34 such claims in the corpus came from — never
        // looked. Filtering at the gate makes the rule a property of an answer rather than of one
        // caller.
        candidates.retain(|candidate| !answer::is_menu(candidate));
        if candidates.is_empty() {
            return Err("the answer carried only a disambiguation page");
        }
        // `Dogma` the film against the concept, `Micah` the prophet against the given name.
        let before = candidates.len();
        candidates
            .retain(|candidate| !types::incompatible(&entity.entity_type, &candidate.classes));
        if before > 0 && candidates.is_empty() {
            return Err("every candidate is a different kind of thing");
        }
        Ok(candidates)
    }

    /// [`parse`], and *why* it refused. `reason` is empty when a claim came back.
    ///
    /// The refusal is recorded where it happens rather than inferred afterwards: an inspector that
    /// guessed at the gate would be a second implementation of the gates.
    pub fn parse_explained(
        &self,
        entity: &model::knowledge_entity::Model,
        request: &Request,
        raw: &Raw,
        reason: &mut &'static str,
    ) -> Vec<Claim> {
        if !raw.outcome.answered() {
            *reason = "the answer carried no candidate";
            return Vec::new();
        }
        let candidates = match self.admit(entity, raw) {
            Ok(candidates) => candidates,
            Err(gate) => {
                *reason = gate;
                return Vec::new();
            }
        };
        // A property match *is* the identity: an item claiming this handle is the account itself.
        if let Some(property) = request.kind.property() {
            match candidates.first() {
                Some(candidate) => {
                    return vec![answer::handle_claim(candidate, property, HANDLE_CONFIDENCE)];
                }
                None => {
                    *reason = "the handle answer carried no item";
                    return Vec::new();
                }
            }
        }
        match &request.kind {
            Kind::Title => policy::parse_title(entity, &candidates, request, reason),
            Kind::Search => policy::parse_search(entity, candidates, request, reason),
            // The handle rung answered above; a label — or anything that is not one of the three —
            // is a ranking question.
            Kind::Label | Kind::Handle(_) => policy::decide(entity, &candidates, request, reason),
        }
    }

    /// Turn answers into claims. First request to speak wins for an entity, which is why `plan`
    /// orders them: a handle is a stronger statement than a title, and a title than a label.
    async fn collect(
        &self,
        entities: &[model::knowledge_entity::Model],
        plans: &[(usize, Request)],
        answers: &Answers,
        gateway: &Gateway<'_>,
        claims: &mut Vec<(i32, Claim)>,
        resolved: &mut HashSet<i32>,
    ) {
        for (index, request) in plans {
            let entity = &entities[*index];
            if resolved.contains(&entity.id) {
                continue;
            }
            let key = (
                request.provider.to_string(),
                request.kind.clone(),
                request.shape.clone(),
                request.query.clone(),
            );
            let Some(raw) = answers.get(&key) else {
                continue;
            };
            let parsed = self.parse(entity, request, raw);
            if parsed.is_empty() {
                // The answer was not wrong, or at least we cannot show it was — only unhelpful.
                // Re-ask it sooner than the default week: a stale negative is indistinguishable from
                // a fact, and a degraded answer looks exactly like a real one.
                gateway.unproductive(request).await;
                continue;
            }
            resolved.insert(entity.id);
            claims.extend(parsed.into_iter().map(|claim| (entity.id, claim)));
        }
    }
}

#[async_trait::async_trait]
impl Resolver for Wikidata {
    fn id(&self) -> &'static str {
        "wikidata"
    }

    /// Handles, titles and exact labels in one pass, then a search pass for whatever is left.
    /// The engine sees only the claims that come back.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Claim)> {
        let mut claims = Vec::new();
        let mut resolved: HashSet<i32> = HashSet::new();

        // Two stages, one loop. The boundary lives here because no other authority knows that search
        // is the expensive route, and the fallback must run offline too: `resolve_requests` decides
        // what to fetch, so it serves cached search answers and silently fetches nothing — skipping
        // it once threw those answers away, and an offline rebuild then reproduced less than the
        // cache held.
        let stages: [fn(&Wikidata, &model::knowledge_entity::Model) -> Vec<Request>; 2] =
            [Wikidata::plan, Wikidata::plan_fallback];
        for plan in stages {
            let plans: Vec<(usize, Request)> = entities
                .iter()
                .enumerate()
                .filter(|(_, entity)| !resolved.contains(&entity.id))
                .flat_map(|(index, entity)| {
                    plan(self, entity)
                        .into_iter()
                        .map(move |request| (index, request))
                })
                .collect();
            if plans.is_empty() {
                continue;
            }
            let requests: Vec<Request> = plans.iter().map(|(_, request)| request.clone()).collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            self.collect(
                entities,
                &plans,
                &answers,
                gateway,
                &mut claims,
                &mut resolved,
            )
            .await;
        }

        // Last resort, after every corroborating question has been asked and refused: the article
        // that bears this exact name. It is the weakest thing the ladder can say, so it speaks last.
        let fallback: Vec<(usize, Request)> = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| !resolved.contains(&entity.id))
            .flat_map(|(index, entity)| {
                Wikidata::plan(self, entity)
                    .into_iter()
                    .filter(|request| request.kind == Kind::Title)
                    .map(move |request| (index, request))
            })
            .collect();
        if !fallback.is_empty() {
            let requests: Vec<Request> = fallback
                .iter()
                .map(|(_, request)| request.clone())
                .collect();
            let answers = gateway.resolve_requests(self, &requests).await;
            for (index, request) in fallback {
                let entity = &entities[index];
                if resolved.contains(&entity.id) {
                    continue;
                }
                let key = (
                    request.provider.to_string(),
                    request.kind.clone(),
                    request.shape.clone(),
                    request.query.clone(),
                );
                let Some(raw) = answers.get(&key) else {
                    continue;
                };
                let Ok(candidates) = self.admit(entity, raw) else {
                    continue;
                };
                if let Some(claim) = policy::surface_only(entity, &candidates, &request) {
                    resolved.insert(entity.id);
                    claims.push((entity.id, claim));
                }
            }
        }

        claims
    }
}

#[async_trait::async_trait]
impl Transport for Wikidata {
    /// The search endpoint takes one query per call; only SPARQL batches.
    fn batch_size(&self, kind: &Kind) -> usize {
        match kind {
            Kind::Search => 1,
            Kind::Title => 50,
            _ => BATCH,
        }
    }

    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
        let Some(first) = requests.first() else {
            return Ok(Vec::new());
        };
        match first.kind {
            Kind::Title => return self.titles(requests).await,
            Kind::Search => {
                let mut answers = Vec::with_capacity(requests.len());
                for request in requests {
                    answers.push((request.query.clone(), self.search(&request.query).await?));
                }
                return Ok(answers);
            }
            Kind::Label | Kind::Handle(_) => {}
        }
        self.sparql(requests).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::identity::cache::Outcome;
    use crate::test_support::db_or_skip;
    use serde_json::{Value, json};

    fn request(kind: Kind, query: &str) -> Request {
        Request {
            provider: "wikidata",
            shape: Wikidata::shape(&kind),
            kind,
            query: query.to_string(),
        }
    }

    fn found(candidates: Value) -> Raw {
        Raw {
            outcome: Outcome::Found,
            candidates: serde_json::from_value(candidates).expect("fixture candidates"),
        }
    }

    fn engine() -> Wikidata {
        Wikidata::new().expect("client should build")
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
        let claims = engine().parse(
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
        let claims = engine().parse(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(Kind::Title, "The Bounty Hunter Wars"),
            &ok,
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].id, "Q16010557");

        // A title answer with no label is not a mismatch — the description still has to corroborate.
        let unlabelled = found(json!([{
            "id": "Q42574",
            "label": null,
            "description": "Canadian filmmaker and explorer (born 1954)",
        }]));
        let claims = engine().parse(
            &entity(
                "James Cameron",
                Some("Canadian filmmaker who served as executive producer on Point Break"),
            ),
            &request(Kind::Title, "James Cameron"),
            &unlabelled,
        );
        assert_eq!(claims.len(), 1, "a label-less answer can still be right");
        assert_eq!(claims[0].id, "Q42574");
    }

    #[test]
    fn an_exact_label_match_is_claimed() {
        let claims = engine().parse(
            &entity("Train Dreams", None),
            &request(Kind::Label, "Train Dreams"),
            &found(json!([{"id": "Q185971", "label": "Train Dreams", "description": "novella"}])),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].id, "Q185971");
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
        let claims = engine().parse(
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
        let claims = engine().parse(
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
        let claims = engine().parse(
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
        let claims = engine().parse(
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
        let claims = engine().parse(
            &entity("Sunset Boulevarde", None),
            &request(Kind::Label, "Sunset Boulevarde"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard"}])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    #[test]
    fn a_handle_match_is_stronger_than_a_label_match() {
        let claims = engine().parse(
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
        let claims = engine().parse(
            &entity(
                "Sunset Boulevard",
                Some("1950 American film noir directed by Billy Wilder about a faded silent-screen star"),
            ),
            &request(Kind::Label, "Sunset Boulevard"),
            &found(candidates),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].id, "Q2");
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
        let claims = engine().parse(
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
            claims.iter().all(|claim| claim.id != "Q79035760"),
            "a one-word match must not carry a claim: {claims:?}"
        );
    }

    /// With no candidate fitting the description, the illumination's hint decides.
    #[test]
    fn the_hint_decides_when_nothing_fits_the_description() {
        let mut view = entity("Stagecoach", Some("a western"));
        view.search_query = Some("Stagecoach 1939 film".to_string());
        let claims = engine().parse(
            &view,
            &request(Kind::Label, "Stagecoach"),
            &found(json!([
                {"id": "Q1", "label": "Stagecoach", "description": "1939 film directed by John Ford"},
                {"id": "Q2", "label": "Stagecoach", "description": "1986 film"},
                {"id": "Q3", "label": "Stagecoach", "description": "a horse-drawn vehicle"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].id, "Q1");
        assert_eq!(claims[0].method, "search_hint");
        assert!(claims[0].confidence <= 0.85, "hint claims are capped lower");
    }

    /// Two candidates fit our description equally: the hint must not break that tie. A hint that
    /// could pick one is still our own text echoing back, not independent evidence.
    #[test]
    fn the_hint_does_not_break_a_tie_between_qualifying_candidates() {
        let mut view = entity("Dune", Some("1965 science fiction novel by Frank Herbert"));
        view.search_query = Some("Dune novel Frank Herbert".to_string());
        let claims = engine().parse(
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
        let claims = engine().parse(
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
        let claims = engine().parse(
            &entity("Dune", Some("1965 science fiction novel by Frank Herbert")),
            &request(Kind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].id, "Q1");
        assert_eq!(claims[0].method, "exact_label");
    }

    #[test]
    fn planning_prefers_the_handle_then_a_title_then_the_label() {
        let mut entity =
            crate::test_support::fixtures::stub_entity(1, "Alexey Kutepov", "real_person");
        entity.search_query = Some("Tsoding".to_string());
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("tsoding".to_string());
        let plans = engine().plan(&entity);
        assert_eq!(plans.len(), 3);
        assert_eq!(plans[0].kind, Kind::Handle("P2002"));
        // A title is unique where a label is not, so it is asked first and wins the claim.
        assert_eq!(plans[1].kind, Kind::Title);
        assert_eq!(plans[2].kind, Kind::Label);

        // Stage two searches the hint first — the disambiguating phrase — then the bare label, which
        // is the lookup blind spot (`J.R.R. Tolkien` vs `J. R. R. Tolkien`).
        let fallback = engine().plan_fallback(&entity);
        assert_eq!(fallback.len(), 2, "{fallback:?}");
        assert_eq!(fallback[0].kind, Kind::Search);
        assert_eq!(fallback[0].query, "Tsoding");
        assert_eq!(fallback[1].query, "Alexey Kutepov");
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
        let plans = engine().plan_fallback(&target);
        assert_eq!(plans.len(), 2, "{plans:?}");
        assert_eq!(plans[0].query, "Commission of Tears Antonio Lobo Antunes");
        assert_eq!(plans[1].query, "Commission of Tears");
    }

    /// A hint that only repeats the name is not a second request.
    #[test]
    fn a_hint_equal_to_the_label_is_not_asked_twice() {
        let mut target = entity("Tsoding", None);
        target.search_query = Some("  TSODING ".to_string());
        let plans = engine().plan_fallback(&target);
        assert_eq!(plans.len(), 1, "{plans:?}");
        assert_eq!(plans[0].query, "Tsoding");
    }

    /// Wikidata labels `J. R. R. Tolkien` with spaces; our label has none. Search finds it, and the
    /// surface comparison ignores the difference.
    #[test]
    fn a_search_answer_is_accepted_on_a_punctuation_variant() {
        let claims = engine().parse(
            &entity("J.R.R. Tolkien", Some("English author, philologist, and academic")),
            &request(Kind::Search, "J.R.R. Tolkien"),
            &found(json!([
                {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"},
                {"id": "Q3805090", "label": "J.R.R. Tolkien's The Lord of the Rings, Vol. I", "description": "DOS game"}
            ])),
        );
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].id, "Q892");
        assert_eq!(claims[0].method, "search_lookup");
    }

    /// A transliteration difference is a spelling of the same name, not a different name:
    /// `Dostoyevsky` is how Wikidata spells him, `Dostoevsky` is how the capture did.
    #[test]
    fn a_search_answer_with_a_misspelt_label_is_ranked() {
        let claims = engine().parse(
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
        assert_eq!(claims[0].id, "Q991");
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
            let claims = engine().parse(
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
        let claims = engine().parse(
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
        assert_eq!(claims[0].id, "Q261122");
        assert_eq!(claims[0].method, "wikipedia_title");
    }

    /// A title that lands on a disambiguation page is a menu, not a referent.
    #[test]
    fn a_title_that_is_a_disambiguation_page_is_refused() {
        let claims = engine().parse(
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
        let claims = engine().parse(
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
        let claims = engine().parse(
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

    /// The ladder, driven end to end. Every answer is already in the cache, so nothing is fetched:
    /// the point is that `resolve` consults the title before the label, and only searches for the
    /// entity the first pass left unresolved.
    #[tokio::test]
    async fn resolve_prefers_a_title_and_searches_only_what_is_left() {
        use crate::knowledge::identity::Options;
        use crate::knowledge::identity::cache::{Answer, Cache};

        db_or_skip!(db);
        let cache = Cache::new(&db.conn);

        let seed = |kind: Kind, query: &str, raw: Raw| {
            let answer = Answer {
                outcome: raw.outcome,
                payload: Some(crate::knowledge::identity::cache::payload_of(
                    &raw.candidates,
                )),
            };
            (kind, query.to_string(), answer)
        };

        let title_wins = found(json!([
            {"id": "Q2", "label": "Train Dreams (novella)", "description": "novella by Denis Johnson"}
        ]));
        let label_loses = found(json!([
            {"id": "Q1", "label": "Train Dreams", "description": "something else entirely"}
        ]));
        let nothing = Raw {
            outcome: Outcome::Empty,
            candidates: Vec::new(),
        };
        let by_search = found(json!([
            {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"}
        ]));

        // The title answers for the first entity, so its label must never be consulted — and a
        // different QID there makes the ordering visible.
        for (kind, query, answer) in [
            seed(Kind::Title, "Train Dreams", title_wins),
            seed(Kind::Label, "Train Dreams", label_loses),
            // The second entity misses both lookups and has to be searched for.
            seed(Kind::Title, "J.R.R. Tolkien", nothing.clone()),
            seed(Kind::Label, "J.R.R. Tolkien", nothing),
            seed(Kind::Search, "J.R.R. Tolkien", by_search),
        ] {
            cache
                .put(
                    "wikidata",
                    &kind.to_key(),
                    &Wikidata::shape(&kind),
                    &query,
                    &answer,
                )
                .await
                .unwrap();
        }

        let entities = vec![entity("Train Dreams", Some("novella by Denis Johnson")), {
            let mut second = entity("J.R.R. Tolkien", None);
            second.id = 2;
            second.description = Some("English author, philologist, and academic".to_string());
            second
        }];

        let options = Options {
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let gateway = Gateway::new(&cache, &options);
        let claims = engine().resolve(&entities, &gateway).await;

        let ids: Vec<(i32, &str)> = claims
            .iter()
            .map(|(id, claim)| (*id, claim.id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q2"), (2, "Q892")], "{claims:?}");
        assert_eq!(
            gateway.fetched(),
            0,
            "everything was answered from the cache"
        );
    }

    /// The exact title is the last resort, not the first answer. `Orthodoxy` the article is the
    /// concept; the label rung, which carries the book, must get to speak first. And when no rung
    /// can corroborate anything — the `Paris` shape, no description, no hint — the title still
    /// answers, because a surface this exact is better than nothing at all.
    #[tokio::test]
    async fn a_surface_only_title_waits_for_every_other_rung() {
        use crate::knowledge::identity::Options;
        use crate::knowledge::identity::cache::{Answer, Cache};

        db_or_skip!(db);
        let cache = Cache::new(&db.conn);

        let seed = |kind: Kind, query: &str, raw: Raw| {
            let answer = Answer {
                outcome: raw.outcome,
                payload: Some(crate::knowledge::identity::cache::payload_of(
                    &raw.candidates,
                )),
            };
            (kind, query.to_string(), answer)
        };
        let nothing = Raw {
            outcome: Outcome::Empty,
            candidates: Vec::new(),
        };

        let concept = json!({
            "id": "Q100",
            "label": "Orthodoxy",
            "description": "adherence to the actual accepted belief, especially in religion",
            "classes": []
        });
        for (kind, query, answer) in [
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
        ] {
            cache
                .put(
                    "wikidata",
                    &kind.to_key(),
                    &Wikidata::shape(&kind),
                    &query,
                    &answer,
                )
                .await
                .unwrap();
        }

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

        let options = Options {
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let gateway = Gateway::new(&cache, &options);
        let claims = engine().resolve(&entities, &gateway).await;

        let ids: Vec<(i32, &str)> = claims
            .iter()
            .map(|(id, claim)| (*id, claim.id.as_str()))
            .collect();
        assert_eq!(ids, vec![(1, "Q200"), (2, "Q300")], "{claims:?}");
        assert_eq!(gateway.fetched(), 0);
    }
}
