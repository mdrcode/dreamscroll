//! Wikidata as an evidence provider.
//!
//! Three request shapes, all identity:
//!
//! | kind | how | batchable | why |
//! | --- | --- | --- | --- |
//! | `label` | SPARQL `VALUES` over the labels | yes | exact label → item, with descriptions and `P31` |
//! | `handle:P…` | SPARQL `VALUES` over the handles | yes | a platform account *is* an identity (`P2002`, `P2003`, …) |
//! | `search` | `wbsearchentities` | no | full-text fallback, only for cores the first stage missed |
//!
//! `plan` and `parse` are pure, so the whole decision can be tested from a fixture and re-run over
//! cached payloads for free. Only `fetch` touches the network, and only the executor calls it.

use serde_json::{json, Value};

use super::{Claim, CoreView, Engine, FetchError, Raw, Request};
use crate::knowledge::cache::Outcome;

const SPARQL_ENDPOINT: &str = "https://query.wikidata.org/sparql";
const USER_AGENT: &str = "dreamscroll-knowledge/0.1 (https://dreamscroll.ai)";
const LANGUAGE: &str = "en";
const SEARCH_LIMIT: usize = 10;

/// A platform account is an identity, and each platform is a different property. Adding one is a row
/// in this table, not a new provider.
///
/// Every property here was verified against Wikidata on 2026-10-31. Unmapped platforms (`threads`,
/// `reddit`, `other`) are deliberate: a wrong property would produce confident, fabricated
/// identities, which is worse than leaving the core unresolved.
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
    client: reqwest::Client,
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
        let mut hasher = blake3::Hasher::new();
        hasher.update(
            format!("wikidata/v2|{kind}|lang={LANGUAGE}|limit={SEARCH_LIMIT}").as_bytes(),
        );
        let hex = hasher.finalize().to_hex();
        hex.as_str()[..12].to_string()
    }

    /// Last resort: the illumination's own disambiguation, consulted *only* when the description
    /// cannot decide. It is weaker evidence than the capture's reading, so its claims are capped
    /// lower, and it can never overrule a description that resolved the case — which is what keeps
    /// the `Sunset Boulevard` failure (the hint lifting the musical adaptation above the film) from
    /// recurring, because there the description decides first.
    fn by_hint(&self, core: &CoreView, candidates: &[Candidate]) -> Vec<Claim> {
        let Some(hint) = hint_tokens(core) else {
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
                    exact_label: true,
                }
            })
            .collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
        let best = &ranked[0];
        let runner_up = ranked.get(1).map(|other| other.score).unwrap_or(0.0);
        if best.score < CLAIM_BAR || best.score - runner_up < CLAIM_MARGIN {
            return Vec::new();
        }
        vec![claim(best.candidate, "search_hint", confidence(best.score).min(0.85))]
    }

    fn kind_for_handle(platform: &str) -> Option<String> {
        handle_property(platform).map(|property| format!("handle:{property}"))
    }

    fn property_of(kind: &str) -> Option<&str> {
        kind.strip_prefix("handle:")
    }
}

#[async_trait::async_trait]
impl Engine for Wikidata {
    fn id(&self) -> &'static str {
        "wikidata"
    }

    fn plan(&self, core: &CoreView) -> Vec<Request> {
        let mut requests = Vec::new();
        if let Some((platform, handle)) = &core.handle
            && let Some(kind) = Self::kind_for_handle(platform)
        {
            requests.push(Request {
                provider: "wikidata",
                shape: Self::shape(&kind),
                kind,
                query: handle.trim().to_string(),
            });
        }
        let label = core.label.trim();
        if !label.is_empty() {
            requests.push(Request {
                provider: "wikidata",
                shape: Self::shape("label"),
                kind: "label".to_string(),
                query: label.to_string(),
            });
        }
        requests
    }

    /// No second request. `search_query` is disambiguation *evidence* for ranking, not a lookup:
    /// Wikidata's entity search is a label matcher, and a string like "Sunset Boulevard 1950 film"
    /// returns nothing from it. Genuine full text would need `list=search` and a page→QID hop, which
    /// is deferred until the label path has been measured over the corpus.
    fn plan_fallback(&self, _core: &CoreView) -> Vec<Request> {
        Vec::new()
    }

    fn parse(&self, core: &CoreView, request: &Request, raw: &Raw) -> Vec<Claim> {
        if raw.outcome != Outcome::Found {
            return Vec::new();
        }
        let Some(payload) = &raw.payload else {
            return Vec::new();
        };
        let candidates: Vec<Candidate> = payload
            .get("candidates")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(Candidate::from_json).collect())
            .unwrap_or_default();
        if candidates.is_empty() {
            return Vec::new();
        }

        // A property match *is* the identity: an item claiming this handle is the account, not
        // something that merely looks like it.
        if Self::property_of(&request.kind).is_some() {
            return candidates
                .first()
                .map(|candidate| claim(candidate, "p2002", 0.95))
                .into_iter()
                .collect();
        }

        let mut ranked = rank(core, &candidates);
        if ranked.is_empty() {
            return Vec::new();
        }

        // One exact label and nothing else claiming it: that is identity, even with no text to
        // corroborate it.
        if ranked.len() == 1 && ranked[0].exact_label {
            return vec![claim(ranked[0].candidate, "exact_label", 0.75)];
        }

        let best = ranked.remove(0);
        let runner_up = ranked.first().map(|other| other.fit).unwrap_or(0.0);

        // Otherwise a claim must clear the bar *and* beat the runner-up. Ambiguity is not a reason to
        // give up — `Sunset Boulevard` has eleven candidates and the description picks the film — but
        // a near-tie is.
        // The decision is on the *evidence*, not the blend. The name is why this candidate is here at
        // all, so requiring a blended score quietly demanded a fixed amount of text overlap — which
        // real, terse Wikidata descriptions cannot supply ("German-born theoretical physicist" vs a
        // capture's six-word reading of Einstein).
        if best.fit < FIT_FLOOR {
            return self.by_hint(core, &candidates);
        }
        // Two candidates fit and neither wins: genuine ambiguity. The hint must not break this tie.
        if best.fit - runner_up < FIT_MARGIN {
            return Vec::new();
        }
        let method = if best.exact_label { "exact_label" } else { "ranked" };
        vec![claim(best.candidate, method, confidence(best.score))]
    }

    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
        let Some(first) = requests.first() else {
            return Ok(Vec::new());
        };
        let query = if let Some(property) = Self::property_of(&first.kind) {
            sparql_for_handles(requests, property)
        } else {
            sparql_for_labels(requests)
        };
        let response = self
            .client
            .post(SPARQL_ENDPOINT)
            .header("Accept", "application/sparql-results+json")
            .form(&[("query", query.as_str()), ("format", "json")])
            .send()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;

        let status = response.status();
        // WDQS throttles with 429 and a Retry-After, and bans clients that ignore it.
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(60);
            return Err(FetchError::RateLimited { retry_after_secs });
        }
        if !status.is_success() {
            return Err(FetchError::Failed(format!("wikidata sparql returned {status}")));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;

        // Every requested query gets an answer, found or empty — so the cache records the negative
        // instead of the executor asking again next run.
        let mut answers: Vec<(String, Raw)> = requests
            .iter()
            .map(|request| {
                (
                    request.query.clone(),
                    Raw {
                        outcome: Outcome::Empty,
                        payload: None,
                    },
                )
            })
            .collect();
        let mut grouped: std::collections::HashMap<String, Vec<Candidate>> =
            std::collections::HashMap::new();
        for row in body
            .pointer("/results/bindings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(query) = row.get("query").and_then(|v| v.get("value")).and_then(Value::as_str)
            else {
                continue;
            };
            let Some(item) = row.get("item").and_then(|v| v.get("value")).and_then(Value::as_str)
            else {
                continue;
            };
            let Some(id) = item.rsplit('/').next() else {
                continue;
            };
            let candidate = Candidate {
                id: id.to_string(),
                label: row
                    .get("itemLabel")
                    .and_then(|v| v.get("value"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                description: row
                    .get("itemDescription")
                    .and_then(|v| v.get("value"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                classes: Vec::new(),
            };
            let entry = grouped.entry(query.to_string()).or_default();
            if !entry.iter().any(|existing| existing.id == candidate.id) {
                entry.push(candidate);
            }
            if let Some(class) = row.get("type").and_then(|v| v.get("value")).and_then(Value::as_str)
                && let Some(class_id) = class.rsplit('/').next()
                && let Some(existing) = entry.last_mut()
            {
                existing.classes.push(class_id.to_string());
            }
        }

        for (query, candidates) in grouped {
            if let Some(slot) = answers.iter_mut().find(|(key, _)| *key == query) {
                slot.1 = Raw {
                    outcome: Outcome::Found,
                    payload: Some(json!({
                        "candidates": candidates.iter().map(Candidate::to_json).collect::<Vec<_>>()
                    })),
                };
            }
        }
        Ok(answers)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    id: String,
    label: Option<String>,
    description: Option<String>,
    classes: Vec<String>,
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

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "description": self.description,
            "classes": self.classes,
        })
    }
}

fn sparql_for_labels(requests: &[Request]) -> String {
    let values = requests
        .iter()
        .map(|request| format!("\"{}\"@{LANGUAGE}", escape(&request.query)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        // No `P31` here on purpose: classes are not used by the ranking, and the extra join made a
        // 50-label batch cost enough processing time to hit WDQS's 60-seconds-per-minute budget.
        // They can come from a separate `VALUES ?item` query when type validation needs them.
        "SELECT ?query ?item ?itemLabel ?itemDescription WHERE {{\n\
         VALUES ?query {{ {values} }}\n\
         ?item rdfs:label ?query .\n\
         SERVICE wikibase:label {{ bd:serviceParam wikibase:language \"{LANGUAGE}\". }}\n\
         }}"
    )
}

fn sparql_for_handles(requests: &[Request], property: &str) -> String {
    let values = requests
        .iter()
        .map(|request| format!("\"{}\"", escape(&request.query)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "SELECT ?query ?item WHERE {{\n\
         VALUES ?query {{ {values} }}\n\
         ?item wdt:{property} ?query .\n\
         }}"
    )
}

/// SPARQL string literals: backslash and double quote are the only escapes that matter here.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Case-insensitive, whitespace-insensitive surface comparison.
fn same_surface(a: &str, b: &str) -> bool {
    a.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        == b.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
}

/// How much of the candidate's description must be accounted for by ours before a claim is worth
/// making. Two shared content words usually clears it; one never does.
const FIT_FLOOR: f32 = 0.20;
/// …and it must beat the runner-up by this much. A near-tie is not evidence.
const FIT_MARGIN: f32 = 0.10;
/// Weight of matched evidence at which a claim is half-convincing. A scale, not a threshold: two
/// common words land well below it, two rare ones well above.
const EVIDENCE_SCALE: f32 = 3.0;
/// The hint fallback is decided on the same margin.
const CLAIM_BAR: f32 = 0.60;
const CLAIM_MARGIN: f32 = 0.10;
/// A name blocks; evidence decides. The name is worth enough to matter and not enough to win alone.
const NAME_WEIGHT: f32 = 0.40;
const SUPPORT_WEIGHT: f32 = 0.60;

/// Words that carry no disambiguation: dropping them makes description overlap mean something.
const NOISE: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "his", "her", "its", "was", "were", "who",
    "which", "about", "into", "are", "but", "not", "由", "der", "die", "das", "les", "los", "las",
];

struct Ranked<'a> {
    candidate: &'a Candidate,
    /// The blended score, for ordering and confidence.
    score: f32,
    /// The description evidence on its own. The decision is made on this, because the blend includes
    /// the name — which every candidate here already shares.
    fit: f32,
    exact_label: bool,
}

/// Score every candidate by how well it fits, best first.
///
/// The signal is the capture's description, read two ways: *containment* — how much of what Wikidata
/// says about the candidate is accounted for by our reading of it, which survives our description
/// being long — and symmetric *overlap*, which separates `Dune` (1965 novel) from `Dune` (the series)
/// because the series' description is a superset of ours.
///
/// `search_query` is deliberately not part of the score. Measured 2026-10-31: adding it lifted
/// `Sunset Boulevard (musical film adaptation)` above the 1950 film, because that candidate's
/// description literally contains the hint's words. A hint drawn from what we saw is not independent
/// evidence about which candidate we meant.
fn rank<'a>(core: &CoreView, candidates: &'a [Candidate]) -> Vec<Ranked<'a>> {
    let ours = content_tokens(core.description.as_deref().unwrap_or_default());
    let weights = Weights::over(candidates);

    let mut scored: Vec<Ranked<'a>> = candidates
        .iter()
        .map(|candidate| {
            let exact_label = candidate
                .label
                .as_deref()
                .is_some_and(|label| same_surface(label, &core.label));
            let name = if exact_label {
                1.0
            } else {
                overlap(
                    &content_tokens(candidate.label.as_deref().unwrap_or_default()),
                    &content_tokens(&core.label),
                )
            };
            let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
            let fit = weights.fit(&theirs, &ours);
            Ranked {
                candidate,
                score: NAME_WEIGHT * name + SUPPORT_WEIGHT * fit,
                fit,
                exact_label,
            }
        })
        .collect();
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    scored
}

fn confidence(score: f32) -> f32 {
    (0.50 + 0.45 * score).min(0.95)
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
    }
}

/// The part of `search_query` that the label does not already say.
///
/// This subtraction is the whole point. `"James Davies anthropologist author"` scores a *photograph
/// of James Davies* on "james" and "davies" — words every candidate shares, because they are the
/// name. Only the remainder ("anthropologist", "author") carries information, and that is what the
/// fallback is allowed to weigh.
fn hint_tokens(core: &CoreView) -> Option<Vec<String>> {
    let raw = core.search_query.as_deref()?.trim();
    if raw.is_empty() || same_surface(raw, &core.label) {
        return None;
    }
    let named: std::collections::BTreeSet<String> = content_tokens(&core.label).into_iter().collect();
    let hint: Vec<String> = content_tokens(raw)
        .into_iter()
        .filter(|token| !named.contains(token))
        .collect();
    (!hint.is_empty()).then_some(hint)
}

fn content_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.chars().count() >= 3 && !NOISE.contains(&token.as_str()))
        .map(stem)
        .collect()
}

/// Light suffix stripping, so `painter`, `painting` and `paint` are the same word.
///
/// Measured on `Caravaggio`: the capture says "revolutionized Western **painting**", Wikidata says
/// "Italian **painter**", and without this they share a single token — under a two-token minimum that
/// is zero evidence, so the best possible description scored nothing.
/// Unigrams plus adjacent bigrams. `"first president"` is stronger evidence than `first` and
/// `president` appearing separately: the phrase is a claim about the same thing, whereas two loose
/// words can be coincidence — our description and a *statue's* both contain `united` and `states`.
fn grams(tokens: &[String]) -> Vec<String> {
    let mut grams = tokens.to_vec();
    grams.extend(tokens.windows(2).map(|pair| format!("{} {}", pair[0], pair[1])));
    grams
}

fn stem(token: String) -> String {
    for suffix in ["ings", "ing", "ed", "es", "s"] {
        if token.len() > suffix.len() + 2 && token.ends_with(suffix) {
            return token[..token.len() - suffix.len()].to_string();
        }
    }
    // Agent nouns only when enough of the word remains: `painter` → `paint` is the case this exists
    // for, while `water` → `wat` and `filmmaker` → `filmak` are damage.
    if token.len() > 6 && (token.ends_with("ers") || token.ends_with("er")) {
        let trimmed = if token.ends_with("ers") {
            &token[..token.len() - 3]
        } else {
            &token[..token.len() - 2]
        };
        if trimmed.len() >= 4 {
            return trimmed.to_string();
        }
    }
    token
}

/// Term rarity within the candidate set.
///
/// This is what separates a *subject* from a *location*: `"George Washington"` returns a president
/// and a "bronze statue in Portland, Oregon, United States", and both share two words with the
/// capture — but `united`/`states` appear in a large fraction of the candidates (every American
/// place) while `president` does not. Inverse document frequency, computed over the candidates the
/// query itself returned, weights them accordingly. No hand-written stoplist, no global tuning.
struct Weights {
    documents: f32,
    frequency: std::collections::BTreeMap<String, usize>,
}

impl Weights {
    fn over(candidates: &[Candidate]) -> Self {
        let mut frequency = std::collections::BTreeMap::new();
        for candidate in candidates {
            let tokens = content_tokens(candidate.description.as_deref().unwrap_or_default());
            for gram in grams(&tokens).into_iter().collect::<std::collections::BTreeSet<_>>() {
                *frequency.entry(gram).or_insert(0) += 1;
            }
        }
        Self {
            documents: candidates.len() as f32,
            frequency,
        }
    }

    fn of(&self, gram: &str) -> f32 {
        let df = self.frequency.get(gram).copied().unwrap_or(0) as f32;
        // A phrase is worth more than a word: it is evidence about a relationship, not a co-occurrence.
        let boost = if gram.contains(' ') { 2.0 } else { 1.0 };
        boost * (1.0 + self.documents / (1.0 + df)).ln()
    }

    /// The weight of distinctive material this candidate accounts for, saturating.
    ///
    /// Deliberately *not* normalised by how verbose the candidate's description is. `George
    /// Washington`'s item matches two rare words (`first`, `president`); the statue in Portland
    /// matches two very common ones (`united`, `states`). Dividing by each description's own total
    /// compressed precisely that difference — the president's text carries four further rare words we
    /// do not share (`founding`, `father`, `1732`, `1799`), which lowered its score for being rich.
    fn fit(&self, theirs: &[String], ours: &[String]) -> f32 {
        let theirs = grams(theirs);
        let ours = grams(ours);
        let ours_set: std::collections::BTreeSet<&String> = ours.iter().collect();
        let theirs_set: std::collections::BTreeSet<&String> = theirs.iter().collect();
        let matched: Vec<&&String> = theirs_set.intersection(&ours_set).collect();
        if matched.len() < 2 {
            return 0.0;
        }
        let matched_weight: f32 = matched.iter().map(|t| self.of(t)).sum();
        matched_weight / (matched_weight + EVIDENCE_SCALE)
    }
}

fn overlap(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a: std::collections::BTreeSet<&String> = a.iter().collect();
    let b: std::collections::BTreeSet<&String> = b.iter().collect();
    a.intersection(&b).count() as f32 / a.union(&b).count() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn core(label: &str, description: Option<&str>) -> CoreView {
        CoreView {
            id: 1,
            label: label.to_string(),
            entity_type: "unknown".to_string(),
            description: description.map(str::to_string),
            aliases: Vec::new(),
            search_query: None,
            handle: None,
        }
    }

    #[test]
    fn an_exact_label_match_is_claimed() {
        let claims = engine().parse(
            &core("Train Dreams", None),
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
            &core("Django", None),
            &request("label", "Django"),
            &found(json!([
                {"id": "Q1", "label": "Django", "description": "web framework"},
                {"id": "Q2", "label": "Django", "description": "2021 video game"}
            ])),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    #[test]
    fn a_label_that_only_fuzzily_matches_is_not_claimed() {
        let claims = engine().parse(
            &core("Sunset Boulevard", None),
            &request("label", "Sunset Boulevard"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard", "description": "street"}])),
        );
        // One exact match, so it *is* claimed — the wrong-referent filter is a separate concern
        // (type compatibility), and this asserts we do not invent a match when there is none.
        assert_eq!(claims.len(), 1);

        let none = engine().parse(
            &core("Sunset Boulevarde", None),
            &request("label", "Sunset Boulevarde"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard"}])),
        );
        assert!(none.is_empty());
    }

    #[test]
    fn a_handle_match_is_stronger_than_a_label_match() {
        let claims = engine().parse(
            &core("Tsoding", None),
            &request("handle:P2002", "tsoding"),
            &found(json!([{"id": "Q1234", "label": "Tsoding"}])),
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].method, "p2002");
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
            &core(
                "Sunset Boulevard",
                Some("1950 American film noir directed by Billy Wilder about a faded silent-screen star"),
            ),
            &request("search", "Sunset Boulevard 1950 film"),
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
            &core(
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
        let mut view = core("Stagecoach", Some("a western"));
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
        let mut view = core("Dune", Some("1965 science fiction novel by Frank Herbert"));
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
            &core("Dune", None),
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
            &core("Dune", Some("1965 science fiction novel by Frank Herbert")),
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
    fn an_empty_answer_claims_nothing() {
        let mume = core("MUME", None);
        assert!(engine()
            .parse(&mume, &request("label", "MUME"), &Raw { outcome: Outcome::Empty, payload: None })
            .is_empty());
        assert!(engine()
            .parse(&mume, &request("label", "MUME"), &Raw { outcome: Outcome::Failed, payload: None })
            .is_empty());
    }

    #[test]
    fn planning_prefers_the_handle_and_always_asks_the_label() {
        let core = CoreView {
            id: 1,
            label: "Alexey Kutepov".to_string(),
            entity_type: "real_person".to_string(),
            description: None,
            aliases: Vec::new(),
            search_query: Some("Tsoding".to_string()),
            handle: Some(("x".to_string(), "tsoding".to_string())),
        };
        let plans = engine().plan(&core);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].kind, "handle:P2002");
        assert_eq!(plans[1].kind, "label");

        assert!(engine().plan_fallback(&core).is_empty(), "no second request");
    }

    #[test]
    fn shapes_differ_per_kind() {
        assert_ne!(Wikidata::shape("label"), Wikidata::shape("search"));
        assert_eq!(Wikidata::shape("label"), Wikidata::shape("label"));
    }

    #[test]
    fn sparql_literals_are_escaped() {
        let requests = vec![request("label", "A \"quoted\" name")];
        let query = sparql_for_labels(&requests);
        assert!(query.contains("\\\"quoted\\\""), "{query}");
    }
}
