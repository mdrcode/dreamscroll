//! Transport: the ways Wikidata answers a question, and nothing else.
//!
//! Every function here is one HTTP shape and its response walking — no ranking, no decisions, no
//! knowledge of what a claim means. They live here so `wikidata.rs` can stay about policy.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::super::driver::{FetchError, Raw, Request};
use super::resolver::{Candidate, Wikidata, LANGUAGE, SEARCH_ENDPOINT, WIKIPEDIA_ENDPOINT};
use crate::knowledge::resolve::cache::Outcome;

/// Which query a SPARQL row answers, and the candidate it names. Items arrive as entity URLs, and a
/// repeated item is deduped by the caller.
fn candidate_from_row(row: &Value) -> Option<(String, Candidate)> {
    let field = |key: &str| row.get(key).and_then(|v| v.get("value")).and_then(Value::as_str);
    let query = field("query")?;
    let id = field("item")?.rsplit('/').next()?;
    Some((
        query.to_string(),
        Candidate {
            id: id.to_string(),
            label: field("itemLabel").map(str::to_string),
            description: field("itemDescription").map(str::to_string),
            classes: Vec::new(),
        },
    ))
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

/// The response, or why it is unusable. A 429 is an instruction — honour its `Retry-After`,
/// defaulting to a minute — and any other non-success is a fact about the provider.
fn checked(response: reqwest::Response, what: &str) -> Result<reqwest::Response, FetchError> {
    let status = response.status();
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
        return Err(FetchError::Failed(format!("{what} returned {status}")));
    }
    Ok(response)
}

impl Wikidata {
    /// Full-text search over items, through MediaWiki's own search (CirrusSearch).
    ///
    /// Deliberately not `wbsearchentities`: that is a *prefix* matcher over labels and aliases, so
    /// it returns nothing for a punctuation difference (`J.R.R.` for `J. R. R.`) and ranks a
    /// disambiguation page above the novel for `A Christmas Carol`. On wikidata.org the page titles
    /// *are* QIDs, so the hit list is already a candidate set.
    pub(super) async fn search(&self, query: &str) -> Result<Raw, FetchError> {
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
        if let Some(error) = body.get("error") {
            return Err(FetchError::Failed(format!("search error: {error}")));
        }
        let outcome = if reached_backend {
            Outcome::Found
        } else {
            Outcome::Degraded
        };
        let ids: Vec<String> = body
            .get("query")
            .and_then(|query| query.get("search"))
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
            return Ok(Raw {
                outcome: if reached_backend {
                    Outcome::Empty
                } else {
                    Outcome::Degraded
                },
                payload: None,
            });
        }

        let items = self.items(&ids).await?;
        let candidates: Vec<Value> = ids.iter().filter_map(|id| items.get(id).cloned()).collect();
        if candidates.is_empty() {
            return Ok(Raw {
                outcome: if reached_backend {
                    Outcome::Empty
                } else {
                    Outcome::Degraded
                },
                payload: None,
            });
        }
        Ok(Raw {
            outcome,
            payload: Some(json!({"candidates": candidates})),
        })
    }

    /// Labels, descriptions and types for a set of items, 50 per call.
    pub(super) async fn items(
        &self,
        ids: &[String],
    ) -> Result<std::collections::HashMap<String, Value>, FetchError> {
        use std::collections::HashMap;

        let mut details: HashMap<String, Value> = HashMap::new();
        for chunk in ids.chunks(50) {
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
            for (qid, item) in body
                .get("entities")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                let label = item
                    .get("labels")
                    .and_then(|labels| labels.get(LANGUAGE))
                    .and_then(|label| label.get("value"))
                    .and_then(Value::as_str);
                let description = item
                    .get("descriptions")
                    .and_then(|descriptions| descriptions.get(LANGUAGE))
                    .and_then(|description| description.get("value"))
                    .and_then(Value::as_str);
                let classes: Vec<Value> = item
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
                                    .map(|id| Value::String(id.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                details.insert(
                    qid.clone(),
                    json!({"id": qid, "label": label, "description": description, "classes": classes}),
                );
            }
        }
        Ok(details)
    }

    /// `enwiki title -> item`, following normalisation and redirects, then one batched read for
    /// the labels, descriptions and types the caller needs.
    pub(super) async fn titles(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError> {
        use std::collections::HashMap;

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
        let query = body.get("query");

        let page_items = page_items(query);
        let renamed = renamed_titles(query);

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
                    Raw {
                        outcome: Outcome::Found,
                        payload: Some(json!({"candidates": [candidate]})),
                    },
                ),
                None => (
                    request.query.clone(),
                    Raw {
                        outcome: Outcome::Empty,
                        payload: None,
                    },
                ),
            });
        }
        Ok(answers)
    }
}


const SPARQL_ENDPOINT: &str = "https://query.wikidata.org/sparql";

impl Wikidata {
    /// SPARQL over `VALUES`: the batched label and handle lookups.
    pub(super) async fn sparql(
        &self,
        requests: &[Request],
    ) -> Result<Vec<(String, Raw)>, FetchError> {
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

        let response = checked(response, "wikidata sparql")?;
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
            let Some((query, candidate)) = candidate_from_row(row) else {
                continue;
            };
            let entry = grouped.entry(query).or_default();
            if !entry.iter().any(|existing| existing.id == candidate.id) {
                entry.push(candidate);
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

pub(super) fn sparql_for_labels(requests: &[Request]) -> String {
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
pub(super) fn sparql_for_handles(requests: &[Request], property: &str) -> String {
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
pub(super) fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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
}
