//! Transport: the ways Wikidata answers a question, and nothing else.
//!
//! Every function here is one HTTP shape and its response walking — no ranking, no decisions, no
//! knowledge of what a claim means. They live here so `resolver.rs` can stay about the ladder.

use std::collections::HashMap;

use serde_json::Value;

use super::query;
use super::{Candidate, LANGUAGE, RequestKind};
use crate::fetch::{Answer, BATCH, FetchError, Outcome, Request, Transport};

/// MediaWiki's API: search, entities and page lookups.
const SEARCH_ENDPOINT: &str = "https://www.wikidata.org/w/api.php";
/// The article-title reconciliation. A Wikipedia title is unique where a Wikidata label is not: 213
/// items are labelled `Paris`, but only one page is titled `Paris`.
const WIKIPEDIA_ENDPOINT: &str = "https://en.wikipedia.org/w/api.php";

/// One answer per query, which is what a transport hands a batch back as.
type Batched = Vec<(String, Answer<Vec<Candidate>>)>;

/// How Wikidata identifies us: its API asks for a tool name with a contact URL.
const USER_AGENT: &str = "dreamscroll-knowledge/0.1 (https://dreamscroll.ai)";
/// One call may take this long. An answer here is small, so a call this slow is a stuck socket,
/// not a slow query — the property of the call, not of the ladder that issues it.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The wire: one client, and the four ways this authority answers a question.
///
/// The door (`crate::fetch`) owns the cache, the pace and the budget; this type only speaks.
pub(super) struct Wire {
    client: reqwest::Client,
}

impl Wire {
    /// The one client this authority speaks through, built here so the ladder above names no HTTP
    /// type: how a call is made is the wire's business.
    pub(super) fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()?;
        Ok(Self { client })
    }
}

/// Which query a SPARQL row answers, and the candidate it names. Items arrive as entity URLs, and a
/// repeated item is deduped by the caller.
fn candidate_from_row(row: &Value) -> Option<(String, Candidate)> {
    let field = |key: &str| {
        row.get(key)
            .and_then(|v| v.get("value"))
            .and_then(Value::as_str)
    };
    let query = field("query")?;
    let id = field("item")?.rsplit('/').next()?;
    Some((
        query.to_string(),
        Candidate {
            id: id.to_string(),
            label: field("itemLabel").map(str::to_string),
            description: field("itemDescription").map(str::to_string),
            // `?class` is present only when the query asked for it (the label rung does); an item
            // with several classes answers once per class, and `sparql` merges the rows.
            classes: field("class")
                .and_then(|class| class.rsplit('/').next())
                .map(|class| vec![class.to_string()])
                .unwrap_or_default(),
        },
    ))
}

/// One `wbgetentities` item as a candidate: the English label and description, and the `P31` classes
/// the kind gate reads.
///
/// `classes` is the field a silent failure would cost the most: empty is a legal answer, so a wrong
/// path through `claims` would not fail — it would turn the gate off for every search and title
/// answer and let a wrong QID be claimed.
fn entity_candidate(qid: &str, item: &Value) -> Candidate {
    let language = |key: &str| {
        item.get(key)
            .and_then(|values| values.get(LANGUAGE))
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str)
    };
    let classes = item
        .get("claims")
        .and_then(|claims| claims.get("P31"))
        .and_then(Value::as_array)
        .map(|claims| {
            claims
                .iter()
                .filter_map(|claim| {
                    claim
                        .get("mainsnak")
                        .and_then(|snak| snak.get("datavalue"))
                        .and_then(|value| value.get("value"))
                        .and_then(|value| value.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    Candidate {
        id: qid.to_string(),
        label: language("labels").map(str::to_string),
        description: language("descriptions").map(str::to_string),
        classes,
    }
}

/// MediaWiki reports a failure inside a `200` body, and a body missing the key it must have is just
/// as unreadable. Either one read as "no results" would cache a negative for a week, so both are
/// errors here — the same rule `search` already applied to a degraded backend.
fn api_result<'a>(body: &'a Value, what: &str, key: &str) -> Result<&'a Value, FetchError> {
    if let Some(error) = body.get("error") {
        return Err(FetchError::Failed(format!("{what} error: {error}")));
    }
    body.get(key)
        .ok_or_else(|| FetchError::Failed(format!("{what}: no `{key}` in body: {body}")))
}

/// `page title → wikibase item`, from a `prop=pageprops` answer. Pages that are missing, or that
/// carry no item, are simply absent.
fn page_items(query: Option<&Value>) -> HashMap<String, String> {
    let mut items = HashMap::new();
    let Some(pages) = query
        .and_then(|query| query.get("pages"))
        .and_then(Value::as_object)
    else {
        return items;
    };
    for page in pages.values() {
        if page.get("missing").is_some() {
            continue;
        }
        let (Some(title), Some(qid)) = (
            page.get("title").and_then(Value::as_str),
            page.get("pageprops")
                .and_then(|props| props.get("wikibase_item"))
                .and_then(Value::as_str),
        ) else {
            continue;
        };
        items.insert(title.to_string(), qid.to_string());
    }
    items
}

/// The `normalized` and `redirects` maps from an answer, each `{from, to}`.
fn renamed_titles(query: Option<&Value>) -> HashMap<String, String> {
    let mut renamed = HashMap::new();
    for key in ["normalized", "redirects"] {
        let Some(entries) = query
            .and_then(|query| query.get(key))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            if let (Some(from), Some(to)) = (
                entry.get("from").and_then(Value::as_str),
                entry.get("to").and_then(Value::as_str),
            ) {
                renamed.insert(from.to_string(), to.to_string());
            }
        }
    }
    renamed
}

/// Follow a title through renaming and redirects. Bounded: a redirect chain can loop.
fn follow(title: &str, renamed: &HashMap<String, String>) -> String {
    let mut current = title.to_string();
    for _ in 0..4 {
        match renamed.get(&current) {
            Some(next) if next != &current => current = next.clone(),
            _ => break,
        }
    }
    current
}

/// The provider's `Retry-After`, in seconds, when it sent a usable one.
///
/// The header is optional and may hold an HTTP date rather than a delta. Either way there is no number
/// to wait: reporting one would claim the provider asked for a wait it never asked for.
fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The response, or why it is unusable. A `429` is an instruction — honour its `Retry-After` when it
/// carries one — and any other non-success is a fact about the provider.
fn checked(response: reqwest::Response, what: &str) -> Result<reqwest::Response, FetchError> {
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(FetchError::RateLimited {
            retry_after_secs: retry_after_secs(response.headers()),
        });
    }
    if !status.is_success() {
        return Err(FetchError::Failed(format!("{what} returned {status}")));
    }
    Ok(response)
}

impl Wire {
    /// Full-text search over items, through MediaWiki's own search (CirrusSearch).
    ///
    /// Deliberately not `wbsearchentities`: that is a *prefix* matcher over labels and aliases, so
    /// it returns nothing for a punctuation difference (`J.R.R.` for `J. R. R.`) and ranks a
    /// disambiguation page above the novel for `A Christmas Carol`. On wikidata.org the page titles
    /// *are* QIDs, so the hit list is already a candidate set.
    pub(super) async fn search(&self, query: &str) -> Result<Answer<Vec<Candidate>>, FetchError> {
        let response = self
            .client
            .get(SEARCH_ENDPOINT)
            .query(&[
                ("action", "query"),
                ("format", "json"),
                ("list", "search"),
                ("srsearch", query),
                ("srnamespace", "0"),
                ("srlimit", &super::SEARCH_LIMIT.to_string()),
            ])
            .send()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        let response = checked(response, "wikidata search")?;
        // CirrusSearch sets this header only when it actually queried Elasticsearch. Its absence is
        // the documented signature of a silently degraded search (Phabricator T284478): HTTP 200,
        // `success: 1`, and a short prefix-only result set that looks exactly like a real answer.
        let reached_backend = response.headers().contains_key("x-search-id");
        let body: Value = response
            .json()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        let query = api_result(&body, "wikidata search", "query")?;
        let outcome = if reached_backend {
            Outcome::Found
        } else {
            Outcome::Degraded
        };
        // What "no results" means here: a fact about the corpus only when the backend answered.
        let negative = || {
            Answer::nothing(if reached_backend {
                Outcome::Empty
            } else {
                Outcome::Degraded
            })
        };
        let ids: Vec<String> = query
            .get("search")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row.get("title").and_then(Value::as_str))
                    .filter(|title| {
                        title.strip_prefix('Q').is_some_and(|rest| {
                            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
                        })
                    })
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            // Zero results is exactly what a degraded backend returns. If CirrusSearch never saw the
            // query, "nothing found" is not a fact about the corpus — it is a failed request wearing
            // a 200. Only a search that actually reached the backend may cache a negative.
            return Ok(negative());
        }

        let items = self.items(&ids).await?;
        let candidates: Vec<Candidate> =
            ids.iter().filter_map(|id| items.get(id).cloned()).collect();
        if candidates.is_empty() {
            return Ok(negative());
        }
        Ok(Answer {
            outcome,
            body: Some(candidates),
        })
    }

    /// Labels, descriptions and types for a set of items, [`BATCH`] per call.
    pub(super) async fn items(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, Candidate>, FetchError> {
        let mut details: HashMap<String, Candidate> = HashMap::new();
        for chunk in ids.chunks(BATCH) {
            let response = self
                .client
                .get(SEARCH_ENDPOINT)
                .query(&[
                    ("action", "wbgetentities"),
                    ("format", "json"),
                    ("ids", chunk.join("|").as_str()),
                    ("props", "labels|descriptions|claims"),
                    ("languages", LANGUAGE),
                ])
                .send()
                .await
                .map_err(|error| FetchError::Failed(error.to_string()))?;
            let response = checked(response, "wikidata entities")?;
            let body: Value = response
                .json()
                .await
                .map_err(|error| FetchError::Failed(error.to_string()))?;
            let entities = api_result(&body, "wikidata entities", "entities")?;
            for (qid, item) in entities.as_object().into_iter().flatten() {
                details.insert(qid.clone(), entity_candidate(qid, item));
            }
        }
        Ok(details)
    }

    /// `enwiki title -> item`, following normalisation and redirects, then one batched read for
    /// the labels, descriptions and types the caller needs.
    pub(super) async fn titles(
        &self,
        requests: &[Request<RequestKind>],
    ) -> Result<Batched, FetchError> {
        let titles: Vec<String> = requests
            .iter()
            .map(|request| request.query.trim().to_string())
            .filter(|title| !title.is_empty())
            .collect();
        if titles.is_empty() {
            return Ok(Vec::new());
        }

        let response = self
            .client
            .get(WIKIPEDIA_ENDPOINT)
            .query(&[
                ("action", "query"),
                ("format", "json"),
                ("redirects", "1"),
                ("prop", "pageprops"),
                ("ppprop", "wikibase_item"),
                ("titles", titles.join("|").as_str()),
            ])
            .send()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        let response = checked(response, "wikipedia search")?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        let query = api_result(&body, "wikipedia search", "query")?;

        let page_items = page_items(Some(query));
        let renamed = renamed_titles(Some(query));

        let mut reached: HashMap<String, String> = HashMap::new();
        let mut ids: Vec<String> = Vec::new();
        for title in &titles {
            if let Some(qid) = page_items.get(&follow(title, &renamed)) {
                reached.insert(title.clone(), qid.clone());
                ids.push(qid.clone());
            }
        }
        ids.sort();
        ids.dedup();

        let details = self.items(&ids).await?;

        let mut answers = Vec::with_capacity(requests.len());
        for request in requests {
            let candidate = reached
                .get(request.query.trim())
                .and_then(|qid| details.get(qid));
            answers.push(match candidate {
                Some(candidate) => (
                    request.query.clone(),
                    Answer {
                        outcome: Outcome::Found,
                        body: Some(vec![candidate.clone()]),
                    },
                ),
                None => (request.query.clone(), Answer::nothing(Outcome::Empty)),
            });
        }
        Ok(answers)
    }
}

const SPARQL_ENDPOINT: &str = "https://query.wikidata.org/sparql";

/// Decode a SPARQL `SELECT` body into one answer per requested query.
///
/// A body we cannot read is an error, never a batch of empty answers: every `Empty` leaving here
/// becomes a cached negative, so a refused or malformed response would claim for a week that the item
/// does not exist. `search` guards its body for the same reason.
fn answers_from_sparql(
    requests: &[Request<RequestKind>],
    body: &Value,
) -> Result<Batched, FetchError> {
    let results = api_result(body, "sparql", "results")?;
    let Some(rows) = results.get("bindings").and_then(Value::as_array) else {
        return Err(FetchError::Failed(format!(
            "sparql: no `bindings` in body: {body}"
        )));
    };

    // Every requested query gets an answer, found or empty — so the cache records the negative
    // instead of the executor asking again next run.
    let mut answers: Batched = requests
        .iter()
        .map(|request| (request.query.clone(), Answer::nothing(Outcome::Empty)))
        .collect();
    let mut grouped: HashMap<String, Vec<Candidate>> = HashMap::new();
    for row in rows {
        let Some((query, candidate)) = candidate_from_row(row) else {
            continue;
        };
        let entry = grouped.entry(query).or_default();
        match entry
            .iter_mut()
            .find(|existing| existing.id == candidate.id)
        {
            Some(existing) => existing.classes.extend(candidate.classes),
            None => entry.push(candidate),
        }
    }

    for (query, candidates) in grouped {
        if let Some(slot) = answers.iter_mut().find(|(key, _)| *key == query) {
            slot.1 = Answer {
                outcome: Outcome::Found,
                body: Some(candidates),
            };
        }
    }
    Ok(answers)
}

impl Wire {
    /// SPARQL over `VALUES`: the batched label and handle lookups.
    pub(super) async fn sparql(
        &self,
        requests: &[Request<RequestKind>],
    ) -> Result<Batched, FetchError> {
        let Some(first) = requests.first() else {
            return Ok(Vec::new());
        };
        let query = if let Some(property) = first.kind.property() {
            query::for_handles(requests, property)
        } else {
            query::for_labels(requests)
        };
        let response = self
            .client
            .post(SPARQL_ENDPOINT)
            .header("Accept", "application/sparql-results+json")
            .form(&[("query", query.as_str()), ("format", "json")])
            .send()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;

        let response = checked(response, "wikidata sparql")?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        answers_from_sparql(requests, &body)
    }
}

/// The wire, as the door sees it: which kinds can share a call, and how one call is shaped.
#[async_trait::async_trait]
impl Transport for Wire {
    type Kind = RequestKind;
    type Payload = Vec<Candidate>;

    /// The search endpoint takes one query per call; only SPARQL batches.
    fn batch_size(&self, kind: &Self::Kind) -> usize {
        match kind {
            RequestKind::Search => 1,
            RequestKind::Title => 50,
            _ => BATCH,
        }
    }

    /// A title lookup reads the page, then the items it names; a search reads its hits the same way.
    fn cost(&self, kind: &Self::Kind) -> usize {
        match kind {
            RequestKind::Title | RequestKind::Search => 2,
            _ => 1,
        }
    }

    async fn fetch(
        &self,
        requests: &[Request<Self::Kind>],
    ) -> Result<Vec<(String, Answer<Self::Payload>)>, FetchError> {
        let Some(first) = requests.first() else {
            return Ok(Vec::new());
        };
        match first.kind {
            RequestKind::Title => return self.titles(requests).await,
            RequestKind::Search => {
                let mut answers = Vec::with_capacity(requests.len());
                for request in requests {
                    answers.push((request.query.clone(), self.search(&request.query).await?));
                }
                return Ok(answers);
            }
            RequestKind::Label | RequestKind::Handle(_) => {}
        }
        self.sparql(requests).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redirect_chain_is_followed() {
        let body = serde_json::json!({"query": {
            "normalized": [{"from": "jrr tolkien", "to": "Jrr Tolkien"}],
            "redirects": [{"from": "Jrr Tolkien", "to": "J. R. R. Tolkien"}],
        }});
        let renamed = renamed_titles(body.get("query"));
        assert_eq!(follow("jrr tolkien", &renamed), "J. R. R. Tolkien");
    }

    /// A redirect that points at itself must not spin.
    #[test]
    fn a_self_redirect_is_not_followed() {
        let body = serde_json::json!({"query": {"redirects": [{"from": "Dune", "to": "Dune"}]}});
        let renamed = renamed_titles(body.get("query"));
        assert_eq!(follow("Dune", &renamed), "Dune");
    }

    #[test]
    fn a_sparql_row_names_its_query_and_item() {
        let row = serde_json::json!({
            "query": {"value": "Train Dreams"},
            "item": {"value": "http://www.wikidata.org/entity/Q85810391"},
            "itemLabel": {"value": "Train Dreams"},
            "itemDescription": {"value": "2011 novella"},
        });
        let (query, candidate) = candidate_from_row(&row).expect("a row with an item");
        assert_eq!(query, "Train Dreams");
        assert_eq!(candidate.id, "Q85810391");
        assert_eq!(candidate.description.as_deref(), Some("2011 novella"));
        assert!(candidate_from_row(&serde_json::json!({"query": {"value": "x"}})).is_none());
    }

    /// The classes a search or title answer carries come from here, and an empty list silently
    /// disables the kind gate — a wrong QID would then be claimed. Both reads are asserted: the
    /// English text, and every `P31` value that is an item reference.
    #[test]
    fn an_entity_reads_its_english_text_and_its_classes() {
        let item = serde_json::json!({
            "labels": {"en": {"value": "Dune"}, "fr": {"value": "Dune (roman)"}},
            "descriptions": {"en": {"value": "1965 novel by Frank Herbert"}},
            "claims": {"P31": [
                {"mainsnak": {"datavalue": {"value": {"id": "Q571"}}}},
                {"mainsnak": {"datavalue": {"value": {"id": "Q7725634"}}}},
                // A `novalue` snak asserts the item has no P31, and names no class.
                {"mainsnak": {"snaktype": "novalue"}},
            ]},
        });

        let candidate = entity_candidate("Q190192", &item);

        assert_eq!(candidate.id, "Q190192");
        assert_eq!(candidate.label.as_deref(), Some("Dune"));
        assert_eq!(
            candidate.description.as_deref(),
            Some("1965 novel by Frank Herbert")
        );
        assert_eq!(candidate.classes, vec!["Q571", "Q7725634"]);
    }

    /// An item with no label, description or claims is a candidate with an id: absent is not empty,
    /// and it is not an error.
    #[test]
    fn an_entity_with_nothing_but_an_id_still_reads() {
        let candidate = entity_candidate("Q1", &serde_json::json!({}));
        assert_eq!(candidate.id, "Q1");
        assert_eq!(candidate.label, None);
        assert_eq!(candidate.description, None);
        assert!(candidate.classes.is_empty());
    }

    #[test]
    fn pages_without_an_item_or_that_are_missing_are_skipped() {
        let body = serde_json::json!({"query": {"pages": {
            "1": {"title": "Dune", "pageprops": {"wikibase_item": "Q190192"}},
            "2": {"title": "No Item", "pageprops": {}},
            "3": {"title": "Gone", "missing": ""},
        }}});
        let items = page_items(body.get("query"));
        assert_eq!(items.get("Dune").map(String::as_str), Some("Q190192"));
        assert_eq!(items.len(), 1, "{items:?}");
    }

    fn labels(query: &str) -> Request<RequestKind> {
        Request {
            provider: "wikidata",
            kind: RequestKind::Label,
            shape: "test".to_string(),
            query: query.to_string(),
        }
    }

    /// Only a plain number is a wait we can honour. No header, or an HTTP date, is `None` — not a
    /// default that would report a wait the provider never asked for.
    #[test]
    fn a_retry_after_is_read_only_when_it_is_a_number() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after_secs(&headers), None);

        headers.insert(reqwest::header::RETRY_AFTER, " 45 ".parse().expect("45"));
        assert_eq!(retry_after_secs(&headers), Some(45));

        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2015 07:28:00 GMT".parse().expect("a date"),
        );
        assert_eq!(retry_after_secs(&headers), None);
    }

    /// Both shapes of unreadable body — an `error` and a missing key — must be errors, because either
    /// one read as "no results" becomes a cached negative. The error case carries the key too, so only
    /// the `error` branch can reject it.
    #[test]
    fn an_api_error_or_a_missing_key_is_an_error() {
        let error = serde_json::json!({
            "error": {"code": "internal_api_error", "info": "ok"},
            "query": {"search": []},
        });
        assert!(
            api_result(&error, "search", "query").is_err(),
            "an error body must not pass just because the key is present"
        );

        let empty = serde_json::json!({"batchcomplete": true});
        assert!(api_result(&empty, "search", "query").is_err());

        let ok = serde_json::json!({"query": {"search": []}});
        assert!(api_result(&ok, "search", "query").is_ok());
    }

    /// WDQS answers a refused or timed-out query with `200` and an `error` body. Turning that into
    /// empty answers would cache "does not exist" for a week, so it must be an error instead.
    #[test]
    fn a_sparql_error_is_an_error_and_never_a_negative() {
        let body = serde_json::json!({"error": {"code": "timeout", "info": "Query timed out"}});
        assert!(answers_from_sparql(&[labels("Sten Mk II")], &body).is_err());
    }

    /// A body we cannot read is the same hazard as an error body: nothing in it names a row, so the
    /// requests would all come back `Empty`.
    #[test]
    fn a_body_without_results_is_an_error_too() {
        let body = serde_json::json!({"head": {"vars": ["query", "item"]}});
        assert!(answers_from_sparql(&[labels("Sten Mk II")], &body).is_err());
    }

    /// A genuinely empty answer is not an error: the negative is the point, so it is still cached.
    #[test]
    fn a_real_empty_answer_still_answers_every_request() {
        let body = serde_json::json!({"head": {}, "results": {"bindings": []}});
        let answers = answers_from_sparql(&[labels("Nobody"), labels("Nobody Here")], &body)
            .expect("a well-formed empty answer");
        assert_eq!(answers.len(), 2);
        assert!(
            answers
                .iter()
                .all(|(_, answer)| answer.outcome == Outcome::Empty)
        );
        assert!(answers.iter().all(|(_, answer)| answer.body.is_none()));
    }

    /// Rows group by query, and an item answering once per class merges into one candidate.
    #[test]
    fn rows_group_into_one_answer_per_query() {
        let row = |item: &str, class: &str| {
            serde_json::json!({
                "query": {"value": "C"},
                "item": {"value": format!("http://www.wikidata.org/entity/{item}")},
                "class": {"value": format!("http://www.wikidata.org/entity/{class}")},
            })
        };
        let body = serde_json::json!({"results": {"bindings": [
            row("Q100", "Q1"),
            row("Q100", "Q2"),
            row("Q200", "Q1"),
        ]}});
        let answers = answers_from_sparql(&[labels("C")], &body).expect("rows");
        assert_eq!(answers.len(), 1);
        let (query, answer) = &answers[0];
        assert_eq!(query, "C");
        assert_eq!(answer.outcome, Outcome::Found);
        let candidates = answer.body.as_ref().expect("found rows carry candidates");
        assert_eq!(candidates.len(), 2);
        let first = candidates.iter().find(|c| c.id == "Q100").expect("Q100");
        assert_eq!(first.classes, vec!["Q1".to_string(), "Q2".to_string()]);
    }
}
