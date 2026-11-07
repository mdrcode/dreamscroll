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
//! it goes through `ResolutionContext::resolve_requests`, which keeps the cache, the pace and the budget.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::knowledge::evidence::ClaimBasis;
use crate::knowledge::resolve::{
    Answers, BATCH, Claim, FetchError, Raw, Request, ResolutionContext, Resolver, Transport,
};
use crate::model;

use super::ranking::{
    CLAIM_BAR, CLAIM_MARGIN, FIT_FLOOR, FIT_MARGIN, HANDLE_CONFIDENCE, HINT_CEILING,
    HINT_MIN_SUPPORT, LONE_LABEL_CONFIDENCE, LOOKUP_CONFIDENCE, NAME_WEIGHT, Ranked,
    SUPPORT_WEIGHT, Weights, answer_confidence, content_tokens, hint_tokens, near_surface,
    our_tokens, rank, surface_key,
};

pub(super) const SEARCH_ENDPOINT: &str = "https://www.wikidata.org/w/api.php";
/// Article-title reconciliation. A Wikipedia title is unique where a Wikidata label is not: 213 items
/// are labelled `Paris`, but only one page is titled `Paris`.
pub(super) const WIKIPEDIA_ENDPOINT: &str = "https://en.wikipedia.org/w/api.php";
/// A menu, not a referent. Two classes because Wikidata's own hierarchy defeats equality: `Q22808320`
/// ("Wikimedia human name disambiguation page") is a *subclass* of `Q4167410`, and every one of the 34
/// disambiguation pages the corpus had claimed carried the subclass, none the parent.
const MENUS: [&str; 2] = [
    "Q4167410",  // Wikimedia disambiguation page
    "Q22808320", // Wikimedia human name disambiguation page
];
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
    fn shape(kind: &str) -> String {
        // The version is per kind, so a transport change invalidates exactly the answers it changes.
        // v3 for `search`: MediaWiki full-text instead of `wbsearchentities`.
        let version = match kind {
            "search" => "wikidata/v3-search",
            _ => "wikidata/v2",
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(
            format!("{version}|{kind}|lang={LANGUAGE}|limit={}", super::SEARCH_LIMIT).as_bytes(),
        );
        let hex = hasher.finalize().to_hex();
        hex.as_str()[..12].to_string()
    }

    /// Last resort: the illumination's own disambiguation, consulted *only* when the description
    /// cannot decide. It is weaker evidence than the capture's description, so its claims are capped
    /// lower, and it can never overrule a description that resolved the case — which is what keeps
    /// the `Sunset Boulevard` failure (the hint lifting the musical adaptation above the film) from
    /// recurring, because there the description decides first.
    fn by_hint(
        &self,
        entity: &model::knowledge_entity::Model,
        candidates: &[Candidate],
        reason: &mut &'static str,
    ) -> Vec<Claim> {
        let Some(hint) = hint_tokens(entity) else {
            *reason = "no disambiguation hint to fall back on";
            return Vec::new();
        };
        let mut ranked: Vec<Ranked> = candidates
            .iter()
            .map(|candidate| {
                let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
                // How much of the *disambiguator* this candidate accounts for. A proportion, not an
                // overlap: the hint is short, and every word of it should be findable.
                let matched = hint.iter().filter(|token| theirs.contains(token)).count();
                let support = matched as f32 / hint.len() as f32;
                Ranked {
                    candidate,
                    score: NAME_WEIGHT + SUPPORT_WEIGHT * support,
                    fit: support,
                    // The hint path gates on fit and support: not an exact-label decision.
                    exact_label: false,
                }
            })
            .collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
        let best = &ranked[0];
        let runner_up = ranked.get(1).map(|other| other.score).unwrap_or(0.0);
        // Half the disambiguating words, not a third. One of three cleared the old bar, which is how
        // `Suspicion` (a Matsumoto novel) claimed a Dürrenmatt one on the word "novel" alone.
        if best.fit < HINT_MIN_SUPPORT
            || best.score < CLAIM_BAR
            || best.score - runner_up < CLAIM_MARGIN
        {
            *reason = "the hint did not clear its bar";
            return Vec::new();
        }
        vec![claim(
            best.candidate,
            "search_hint",
            answer_confidence(best.score).min(HINT_CEILING),
        )]
    }
    pub(super) fn property_of(kind: &str) -> Option<&str> {
        kind.strip_prefix("handle:")
    }

    /// Choose a referent from the candidates, or none. Shared by the label path and the search
    /// fallback — the only difference between them is which candidates are admitted.
    fn decide(
        &self,
        entity: &model::knowledge_entity::Model,
        candidates: &[Candidate],
        request: &Request,
        reason: &mut &'static str,
    ) -> Vec<Claim> {
        let mut ranked = rank(entity, candidates);
        if ranked.is_empty() {
            *reason = "no candidate survived the name test";
            return Vec::new();
        }

        // One exact label and nothing else claiming it: that is identity, even with no text to
        // corroborate it.
        if ranked.len() == 1 && ranked[0].exact_label {
            return vec![claim(
                ranked[0].candidate,
                "exact_label",
                LONE_LABEL_CONFIDENCE,
            )];
        }

        let best = ranked.remove(0);
        let runner_up = ranked.first().map(|other| other.fit).unwrap_or(0.0);

        // Otherwise a claim must clear the bar *and* beat the runner-up. Ambiguity is not a reason to
        // give up — `Sunset Boulevard` has eleven candidates and the description picks the film — but
        // a near-tie is.
        // The decision is on the *evidence*, not the blend. The name is why this candidate is here at
        // all, so requiring a blended score quietly demanded a fixed amount of text overlap — which
        // real, terse Wikidata descriptions cannot supply ("German-born theoretical physicist" vs a
        // capture's six-word description of Einstein).
        if best.fit < FIT_FLOOR {
            *reason = "the description does not corroborate";
            return self.by_hint(entity, candidates, reason);
        }
        // Two candidates fit and neither wins: genuine ambiguity. The hint must not break this tie.
        if best.fit - runner_up < FIT_MARGIN {
            *reason = "two candidates fit and neither wins";
            return Vec::new();
        }
        let method = match (request.kind.as_str(), best.exact_label) {
            ("title", _) => "wikipedia_title",
            ("search", true) => "search_lookup",
            ("search", false) => "search_ranked",
            (_, true) => "exact_label",
            (_, false) => "ranked",
        };
        vec![claim(
            best.candidate,
            method,
            answer_confidence(best.score),
        )]
    }

    /// The first pass: a handle, then a title, then an exact label. Pure, so the order is testable
    /// without a network — and public for the same reason: `examples/eval_resolve` re-runs the whole
    /// decision over cached payloads.
    pub fn plan(&self, entity: &model::knowledge_entity::Model) -> Vec<Request> {
        let mut requests = Vec::new();
        if let (Some(platform), Some(handle)) = (&entity.platform, &entity.platform_handle)
            && let Some(property) = handle_property(platform)
        {
            let kind = format!("handle:{property}");
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
                shape: Self::shape("title"),
                kind: "title".to_string(),
                query: label.to_string(),
            });
            requests.push(Request {
                provider: "wikidata",
                shape: Self::shape("label"),
                kind: "label".to_string(),
                query: label.to_string(),
            });
        }
        requests
    }

    /// The second pass: search for the **bare label**, for entities the first pass left unresolved.
    ///
    /// This is a lookup fallback, not a ranking signal. Wikidata labels `J. R. R. Tolkien` and
    /// `D. H. Lawrence` *with spaces*; our exact-label SPARQL sends `J.R.R. Tolkien` and gets nothing,
    /// while search finds Q892 immediately. Measured on 20 random empty-label entities: 4 recoverable
    /// this way.
    ///
    /// The pass boundary lives here, not in the engine: no other authority knows that search is the
    /// expensive route.
    pub fn plan_fallback(&self, entity: &model::knowledge_entity::Model) -> Vec<Request> {
        let label = entity.name.trim();
        if label.is_empty() {
            return Vec::new();
        }
        vec![Request {
            provider: "wikidata",
            shape: Self::shape("search"),
            kind: "search".to_string(),
            query: label.to_string(),
        }]
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
        let Some(mut candidates) = Self::candidates_of(raw) else {
            *reason = "the answer carried no candidate";
            return Vec::new();
        };
        // A menu is not a referent, on *every* rung. This used to be checked in `parse_title` only,
        // and the label rung — which is where all 34 such claims in the corpus came from — never
        // looked. Filtering at the gate makes the rule a property of an answer rather than of one
        // caller.
        candidates.retain(|candidate| !Self::is_menu(candidate));
        if candidates.is_empty() {
            *reason = "the answer carried only a disambiguation page";
            return Vec::new();
        }
        // A property match *is* the identity: an item claiming this handle is the account itself.
        if let Some(property) = Self::property_of(&request.kind) {
            match candidates.first() {
                Some(candidate) => return vec![handle_claim(candidate, property, HANDLE_CONFIDENCE)],
                None => {
                    *reason = "the handle answer carried no item";
                    return Vec::new();
                }
            }
        }
        match request.kind.as_str() {
            "title" => self.parse_title(entity, &candidates, request, reason),
            "search" => self.parse_search(entity, candidates, request, reason),
            _ => self.decide(entity, &candidates, request, reason),
        }
    }

    /// A disambiguation page, or an item that is otherwise a list rather than a thing.
    ///
    /// Two signals because neither is complete: the class list is structural but misses items stored
    /// with no classes at all (three of the corpus's), while the autogenerated description is a
    /// convention that caught all 34.
    fn is_menu(candidate: &Candidate) -> bool {
        candidate
            .classes
            .iter()
            .any(|class| MENUS.contains(&class.as_str()))
            || candidate
                .description
                .as_deref()
                .is_some_and(|text| text.to_lowercase().contains("disambiguation"))
    }

    /// The candidates an answer carries; a degraded answer still has them.
    fn candidates_of(raw: &Raw) -> Option<Vec<Candidate>> {
        if !raw.outcome.answered() {
            return None;
        }
        let rows = raw.payload.as_ref()?.get("candidates")?.as_array()?;
        let candidates: Vec<Candidate> = rows.iter().filter_map(Candidate::from_json).collect();
        (!candidates.is_empty()).then_some(candidates)
    }

    /// A title maps to one article. A disambiguation page maps to none: it is a menu, and claiming
    /// it is the trap that produced `Royal Navy` → an aircraft-carrier fleet.
    fn parse_title(
        &self,
        entity: &model::knowledge_entity::Model,
        candidates: &[Candidate],
        request: &Request,
        reason: &mut &'static str,
    ) -> Vec<Claim> {
        // A title is a *name* lookup, so the answer has to be about that name. Wikipedia redirects
        // otherwise: asking for `The Bounty Hunter Wars` lands on the article for `Boba Fett`, and
        // "one title, one article" then reads as identity. Sharing no word with our name is not an
        // identity, whatever the title said.
        let ours = content_tokens(&entity.name);
        let titled: Vec<Candidate> = candidates
            .iter()
            .filter(|candidate| {
                // No label is not a mismatch: Wikidata sometimes answers with a description only,
                // and `Q42574 · "Canadian filmmaker and explorer"` is exactly the right item for
                // `James Cameron`. `decide` still requires the description to corroborate.
                let Some(label) = candidate.label.as_deref() else {
                    return true;
                };
                if near_surface(&surface_key(label), &surface_key(&entity.name)) {
                    return true;
                }
                content_tokens(label).iter().any(|token| ours.contains(token))
            })
            .cloned()
            .collect();
        if titled.is_empty() {
            *reason = "the title is about another subject";
            return Vec::new();
        }
        self.decide(entity, &titled, request, reason)
    }

    /// A search answer: one corroborated surface is identity, anything else is a ranker question.
    fn parse_search(
        &self,
        entity: &model::knowledge_entity::Model,
        candidates: Vec<Candidate>,
        request: &Request,
        reason: &mut &'static str,
    ) -> Vec<Claim> {
        let ours = surface_key(&entity.name);
        let same: Vec<&Candidate> = candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .label
                    .as_deref()
                    .is_some_and(|label| surface_key(label) == ours)
            })
            .collect();
        // A matching surface is not enough — the description has to corroborate it, exactly as on
        // the label path. Without this, `Audi` (a car brand) claims a *family name* and
        // `A Pattern Language` claims a 2011 master's thesis, both of which match the surface.
        //
        // Anything short of one corroborated surface falls through to the ranker: *zero* is a
        // spelling difference (`J.R.R.` vs `J. R. R.`), *several* is a shared title where only the
        // description can pick the referent.
        if let [candidate] = same.as_slice() {
            let weights = Weights::over(&candidates);
            let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
            let ours = our_tokens(entity);
            let fit = ours
                .iter()
                .map(|tokens| weights.fit(&theirs, tokens))
                .fold(0.0f32, f32::max);
            if fit >= FIT_FLOOR {
                return vec![claim(
                    candidate,
                    "search_lookup",
                    LOOKUP_CONFIDENCE,
                )];
            }
        }
        // A candidate is admissible when it *is* our name, a trivial respelling of it, or one of the
        // two is a superset of the other's content words: `Tomb of Napoleon Bonaparte` vs `Tomb of
        // Napoleon`, `Andriivka, Donetsk Oblast` vs `Andriivka`, `The Justice of Emperor Otto III`
        // vs `Justice of Emperor Otto III: The Beheading…`. The ladder used to admit only the first
        // two, so a name carrying a qualifier never let its description speak — measured on the
        // local corpus, 51 undecided entities had the right item in hand and were refused here.
        //
        // Admission is not a claim. The description must still clear `FIT_FLOOR` and beat the
        // runner-up, which is what keeps `Royal Navy fleet aircraft carrier fleet` off `Royal Navy`.
        let mine = content_tokens(&entity.name);
        let admitted: Vec<Candidate> = candidates
            .iter()
            .filter(|candidate| {
                let Some(label) = candidate.label.as_deref() else {
                    return false;
                };
                if near_surface(&surface_key(label), &ours) {
                    return true;
                }
                // Our name is the label plus a qualifier the authority keeps elsewhere: `Andriivka,
                // Donetsk Oblast`, `Pennsylvania Station (1910–1963)`. The qualifier is ours to
                // carry, not the authority's — and refusing here means the description never
                // gets to separate the six villages that share one name.
                let (name_flat, label_flat) = (
                    entity.name.trim().to_lowercase(),
                    label.trim().to_lowercase(),
                );
                if let Some(rest) = name_flat.strip_prefix(&label_flat) {
                    let rest = rest.trim_start();
                    if rest.starts_with(',') || rest.starts_with('(') {
                        return true;
                    }
                }
                // A one-word name is still a name: `Rare` against `Rare Ltd.`, `Bash` against
                // `GNU Bash`. Requiring both sides to carry two content words excluded exactly the
                // short names, and the description is what decides anyway.
                let theirs = content_tokens(label);
                if theirs.is_empty() || mine.is_empty() {
                    return false;
                }
                let (small, large) = if theirs.len() <= mine.len() {
                    (&theirs, &mine)
                } else {
                    (&mine, &theirs)
                };
                small.iter().all(|token| large.contains(token))
            })
            .cloned()
            .collect();
        if admitted.is_empty() {
            *reason = "nothing in the answer relates to the name";
            // The name nominated nothing, but the hint may still: `Sten Mk IIS suppressed` is a
            // model whose label is just `Sten`, and no name rule will connect the two. Admission is
            // a name test, so the hint path has to be reached before it turns the answer down.
            return self.by_hint(entity, &candidates, reason);
        }
        self.decide(entity, &admitted, request, reason)
    }

    /// Turn answers into claims. First request to speak wins for an entity, which is why `plan`
    /// orders them: a handle is a stronger statement than a title, and a title than a label.
    async fn collect(
        &self,
        entities: &[model::knowledge_entity::Model],
        plans: &[(usize, Request)],
        answers: &Answers,
        ctx: &ResolutionContext<'_>,
        claims: &mut Vec<(i32, Claim)>,
        resolved: &mut HashSet<i32>,
    ) {
        for (index, request) in plans {
            let entity = &entities[*index];
            if resolved.contains(&entity.id) {
                continue;
            }
            let key = (
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
                ctx.unproductive(request).await;
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
        ctx: &ResolutionContext<'_>,
    ) -> Vec<(i32, Claim)> {
        let mut claims = Vec::new();
        let mut resolved: HashSet<i32> = HashSet::new();

        let identity: Vec<(usize, Request)> = entities
            .iter()
            .enumerate()
            .flat_map(|(index, entity)| {
                self.plan(entity)
                    .into_iter()
                    .map(move |request| (index, request))
            })
            .collect();
        let requests: Vec<Request> = identity
            .iter()
            .map(|(_, request)| request.clone())
            .collect();
        let answers = ctx.resolve_requests(self, &requests).await;
        self.collect(entities, &identity, &answers, ctx, &mut claims, &mut resolved)
            .await;

        // The fallback runs offline too: `resolve_requests` decides what to fetch, so it serves the
        // cached search answers and silently fetches nothing. Skipping it here threw those answers
        // away, which made an offline rebuild reproduce only what the first pass found.

        let fallback: Vec<(usize, Request)> = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| !resolved.contains(&entity.id))
            .flat_map(|(index, entity)| {
                self.plan_fallback(entity)
                    .into_iter()
                    .map(move |request| (index, request))
            })
            .collect();
        let requests: Vec<Request> = fallback
            .iter()
            .map(|(_, request)| request.clone())
            .collect();
        let answers = ctx.resolve_requests(self, &requests).await;
        self.collect(entities, &fallback, &answers, ctx, &mut claims, &mut resolved)
            .await;

        claims
    }
}

#[async_trait::async_trait]
impl Transport for Wikidata {
    /// The search endpoint takes one query per call; only SPARQL batches.
    fn batch_size(&self, kind: &str) -> usize {
        if kind == "search" {
            1
        } else if kind == "title" {
            50
        } else {
            BATCH
        }
    }

    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
        let Some(first) = requests.first() else {
            return Ok(Vec::new());
        };
        if first.kind == "title" {
            return self.titles(requests).await;
        }
        if first.kind == "search" {
            let mut answers = Vec::with_capacity(requests.len());
            for request in requests {
                answers.push((request.query.clone(), self.search(&request.query).await?));
            }
            return Ok(answers);
        }

        return self.sparql(requests).await;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Candidate {
    pub(super) id: String,
    pub(super) label: Option<String>,
    pub(super) description: Option<String>,
    pub(super) classes: Vec<String>,
}

impl Candidate {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            label: value.get("label").and_then(Value::as_str).map(str::to_string),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
            classes: value
                .get("classes")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    pub(super) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "description": self.description,
            "classes": self.classes,
        })
    }
}

fn claim(candidate: &Candidate, method: &str, confidence: f32) -> Claim {
    Claim {
        scheme: "wikidata".to_string(),
        id: candidate.id.clone(),
        label: candidate.label.clone(),
        description: candidate.description.clone(),
        classes: candidate.classes.clone(),
        method: method.to_string(),
        confidence,
        basis: ClaimBasis::Text,
    }
}

/// A property match: the item *is* the account. The basis is the handle, which is what stops the
/// answer counting as a second, independent input beside the handle that produced it — however many
/// platforms the property table grows.
fn handle_claim(candidate: &Candidate, property: &str, confidence: f32) -> Claim {
    let mut claim = claim(candidate, property, confidence);
    claim.basis = ClaimBasis::Handle;
    claim
}

#[cfg(test)]
mod tests {
    use crate::knowledge::resolve::cache::Outcome;
    use super::*;
    use crate::test_support::db_or_skip;

    fn request(kind: &str, query: &str) -> Request {
        Request {
            provider: "wikidata",
            kind: kind.to_string(),
            shape: Wikidata::shape(kind),
            query: query.to_string(),
        }
    }

    fn found(candidates: Value) -> Raw {
        Raw {
            outcome: Outcome::Found,
            payload: Some(json!({"candidates": candidates})),
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
        let raw = Raw {
            outcome: Outcome::Found,
            payload: Some(json!({
                "candidates": [{
                    "id": "Q51790",
                    "label": "Boba Fett",
                    "description": "fictional character from Star Wars",
                }]
            })),
        };
        let claims = engine().parse(
            &entity("The Bounty Hunter Wars", Some("a trilogy of Star Wars novels")),
            &request("title", "The Bounty Hunter Wars"),
            &raw,
        );
        assert!(claims.is_empty(), "a redirect is not an identity: {claims:?}");

        // …while a title whose label is our name still claims.
        let ok = Raw {
            outcome: Outcome::Found,
            payload: Some(json!({
                "candidates": [{
                    "id": "Q16010557",
                    "label": "The Bounty Hunter Wars",
                    "description": "novel trilogy by K. W. Jeter",
                }]
            })),
        };
        let claims = engine().parse(
            &entity("The Bounty Hunter Wars", Some("a trilogy of Star Wars novels")),
            &request("title", "The Bounty Hunter Wars"),
            &ok,
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].id, "Q16010557");

        // A title answer with no label is not a mismatch — the description still has to corroborate.
        let unlabelled = Raw {
            outcome: Outcome::Found,
            payload: Some(json!({
                "candidates": [{
                    "id": "Q42574",
                    "label": null,
                    "description": "Canadian filmmaker and explorer (born 1954)",
                }]
            })),
        };
        let claims = engine().parse(
            &entity(
                "James Cameron",
                Some("Canadian filmmaker who served as executive producer on Point Break"),
            ),
            &request("title", "James Cameron"),
            &unlabelled,
        );
        assert_eq!(claims.len(), 1, "a label-less answer can still be right");
        assert_eq!(claims[0].id, "Q42574");
    }

    #[test]
    fn an_exact_label_match_is_claimed() {
        let claims = engine().parse(
            &entity("Train Dreams", None),
            &request("label", "Train Dreams"),
            &found(json!([{"id": "Q185971", "label": "Train Dreams", "description": "novella"}])),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].id, "Q185971");
        assert_eq!(claims[0].method, "exact_label");
        // A unique exact label is identity even with nothing to corroborate it — but it does not
        // reach the confidence a description-backed claim gets.
        assert!((claims[0].confidence - 0.75).abs() < 1e-6, "{:?}", claims[0]);
    }

    #[test]
    fn an_ambiguous_label_is_not_claimed() {
        // The Django case: two items share the surface, so the name cannot decide.
        let claims = engine().parse(
            &entity("Django", None),
            &request("label", "Django"),
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
            &request("label", "Sunset Boulevarde"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard"}])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    #[test]
    fn a_handle_match_is_stronger_than_a_label_match() {
        let claims = engine().parse(
            &entity("Tsoding", None),
            &request("handle:P2002", "tsoding"),
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
            &request("label", "Sunset Boulevard"),
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
            &request("label", "George Washington"),
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
            &request("label", "Stagecoach"),
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
            &request("label", "Dune"),
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
            &request("label", "Dune"),
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
            &request("label", "Dune"),
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
        let mut entity = crate::test_support::fixtures::stub_entity(1, "Alexey Kutepov", "real_person");
        entity.search_query = Some("Tsoding".to_string());
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("tsoding".to_string());
        let plans = engine().plan(&entity);
        assert_eq!(plans.len(), 3);
        assert_eq!(plans[0].kind, "handle:P2002");
        // A title is unique where a label is not, so it is asked first and wins the claim.
        assert_eq!(plans[1].kind, "title");
        assert_eq!(plans[2].kind, "label");

        // Stage two searches for the bare label: that is the lookup blind spot it exists for.
        let fallback = engine().plan_fallback(&entity);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].kind, "search");
        assert_eq!(fallback[0].query, "Alexey Kutepov");
    }

    /// Wikidata labels `J. R. R. Tolkien` with spaces; our label has none. Search finds it, and the
    /// surface comparison ignores the difference.
    #[test]
    fn a_search_answer_is_accepted_on_a_punctuation_variant() {
        let claims = engine().parse(
            &entity("J.R.R. Tolkien", Some("English author, philologist, and academic")),
            &request("search", "J.R.R. Tolkien"),
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
            &request("search", "Fyodor Dostoevsky"),
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
            ("Clawdbot", "an open-source personal AI agent harness", "Clawdbot Pro"),
        ] {
            let claims = engine().parse(
                &entity(name, Some(description)),
                &request("search", name),
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
            &request("title", "Symphony No. 7 (Sibelius)"),
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
            &request("title", "John Grant"),
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
            &request("label", "Robert Chandler"),
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
            &entity("Rogue Heroes", Some("British historical drama television series")),
            &request("label", "Rogue Heroes"),
            &found(json!([{
                "id": "Q115697563",
                "label": "Rogue Heroes",
                "description": "Wikimedia disambiguation page",
                "classes": []
            }])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    #[test]
    fn sparql_literals_are_escaped() {
        let requests = vec![request("label", "A \"quoted\" name")];
        let query = crate::knowledge::resolve::wikidata::transport::sparql_for_labels(&requests);
        assert!(query.contains("\\\"quoted\\\""), "{query}");
    }

    /// The ladder, driven end to end. Every answer is already in the cache, so nothing is fetched:
    /// the point is that `resolve` consults the title before the label, and only searches for the
    /// entity the first pass left unresolved.
    #[tokio::test]
    async fn resolve_prefers_a_title_and_searches_only_what_is_left() {
        use crate::knowledge::resolve::cache::{Answer, Cache};
        use crate::knowledge::resolve::Options;

        db_or_skip!(db);
        let cache = Cache::new(&db.conn);

        let seed = |kind: &str, query: &str, raw: Raw| {
            let answer = Answer {
                outcome: raw.outcome,
                payload: raw.payload,
            };
            (kind.to_string(), query.to_string(), answer)
        };

        let title_wins = found(json!([
            {"id": "Q2", "label": "Train Dreams (novella)", "description": "novella by Denis Johnson"}
        ]));
        let label_loses = found(json!([
            {"id": "Q1", "label": "Train Dreams", "description": "something else entirely"}
        ]));
        let nothing = Raw {
            outcome: Outcome::Empty,
            payload: None,
        };
        let by_search = found(json!([
            {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"}
        ]));

        // The title answers for the first entity, so its label must never be consulted — and a
        // different QID there makes the ordering visible.
        for (kind, query, answer) in [
            seed("title", "Train Dreams", title_wins),
            seed("label", "Train Dreams", label_loses),
            // The second entity misses both lookups and has to be searched for.
            seed("title", "J.R.R. Tolkien", nothing.clone()),
            seed("label", "J.R.R. Tolkien", nothing),
            seed("search", "J.R.R. Tolkien", by_search),
        ] {
            cache
                .put("wikidata", &kind, &Wikidata::shape(&kind), &query, &answer)
                .await
                .unwrap();
        }

        let entities = vec![
            entity("Train Dreams", Some("novella by Denis Johnson")),
            {
                let mut second = entity("J.R.R. Tolkien", None);
                second.id = 2;
                second.description = Some("English author, philologist, and academic".to_string());
                second
            },
        ];

        let options = Options {
            min_interval: std::time::Duration::from_millis(0),
            ..Default::default()
        };
        let ctx = ResolutionContext::new(&cache, &options);
        let claims = engine().resolve(&entities, &ctx).await;

        let ids: Vec<(i32, &str)> = claims.iter().map(|(id, claim)| (*id, claim.id.as_str())).collect();
        assert_eq!(ids, vec![(1, "Q2"), (2, "Q892")], "{claims:?}");
        assert_eq!(ctx.fetched(), 0, "everything was answered from the cache");
    }
}
