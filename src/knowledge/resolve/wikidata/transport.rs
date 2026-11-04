//! Transport: the ways Wikidata answers a question, and nothing else.
//!
//! Every function here is one HTTP shape and its response walking — no ranking, no decisions, no
//! knowledge of what a claim means. They live here so `wikidata.rs` can stay about policy.

use serde_json::{json, Value};

use super::super::driver::{FetchError, Raw, Request};
use super::resolver::{Candidate, Wikidata, LANGUAGE, SEARCH_ENDPOINT, WIKIPEDIA_ENDPOINT};
use crate::knowledge::resolve::cache::Outcome;

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
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(FetchError::RateLimited {
                retry_after_secs: 60,
            });
        }
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Failed(format!("wikidata search returned {status}")));
        }
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
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(FetchError::RateLimited { retry_after_secs: 60 });
            }
            let status = response.status();
            if !status.is_success() {
                return Err(FetchError::Failed(format!("wikidata entities returned {status}")));
            }
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
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(FetchError::RateLimited { retry_after_secs: 60 });
        }
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Failed(format!("wikipedia search returned {status}")));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        let query = body.get("query");

        let mut by_title: HashMap<String, String> = HashMap::new();
        if let Some(pages) = query.and_then(|query| query.get("pages")).and_then(Value::as_object) {
            for page in pages.values() {
                if page.get("missing").is_some() {
                    continue;
                }
                let Some(title) = page.get("title").and_then(Value::as_str) else {
                    continue;
                };
                let Some(qid) = page
                    .get("pageprops")
                    .and_then(|props| props.get("wikibase_item"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                by_title.insert(title.to_string(), qid.to_string());
            }
        }

        // `normalized` then `redirects`, each `{from, to}`; following both gives the article title.
        let mut renamed: HashMap<String, String> = HashMap::new();
        for key in ["normalized", "redirects"] {
            if let Some(entries) = query.and_then(|query| query.get(key)).and_then(Value::as_array) {
                for entry in entries {
                    if let (Some(from), Some(to)) = (
                        entry.get("from").and_then(Value::as_str),
                        entry.get("to").and_then(Value::as_str),
                    ) {
                        renamed.insert(from.to_string(), to.to_string());
                    }
                }
            }
        }

        let mut reached: HashMap<String, String> = HashMap::new();
        let mut ids: Vec<String> = Vec::new();
        for title in &titles {
            let mut current = title.clone();
            for _ in 0..4 {
                match renamed.get(&current) {
                    Some(next) if next != &current => current = next.clone(),
                    _ => break,
                }
            }
            if let Some(qid) = by_title.get(&current) {
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
