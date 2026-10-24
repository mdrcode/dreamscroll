use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::llms::{InferenceMetadata, InferenceOutcome};

use super::service::parse_fields;
use super::{
    DataObject, Embedder, Embedding, QueryParams, SearchHit, SearchResultPage, Searcher, Unit,
    VectorStore, VectorUpsertResult,
};

const DIMS: usize = 128;

/// Deterministic, dependency-free embedding used by throwaway smoke runs.
///
/// This is intentionally not a semantic model. It gives local search a stable
/// vector space so upload -> illumination -> search index -> related previews
/// can run without writing to the shared Vertex collection.
#[derive(Clone, Default)]
pub struct LocalEmbedder;

impl LocalEmbedder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Embedder<Value, Embedding<f32, Unit>> for LocalEmbedder {
    async fn embed_query(&self, query: &str) -> anyhow::Result<Embedding<f32, Unit>> {
        embed_text(query)
    }

    async fn embed_object(
        &self,
        object: Value,
    ) -> anyhow::Result<InferenceOutcome<Embedding<f32, Unit>>> {
        let text = extract_text(&object);
        Ok(InferenceOutcome {
            output: Ok(embed_text(&text)?),
            metadata: InferenceMetadata {
                provider_name: "local".to_string(),
                backend_name: "smoke".to_string(),
                model_id: "local-hash".to_string(),
                ..Default::default()
            },
        })
    }
}

/// In-memory vector store and searcher for one smoke server process.
#[derive(Clone, Default)]
pub struct LocalSearch {
    embeddings: Arc<RwLock<HashMap<String, Embedding<f32, Unit>>>>,
}

#[async_trait::async_trait]
impl Searcher<Embedding<f32, Unit>> for LocalSearch {
    async fn search_text(
        &self,
        text_query: &str,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage> {
        self.search_embedding(&embed_text(text_query)?, params)
            .await
    }

    async fn search_embedding(
        &self,
        embedding_query: &Embedding<f32, Unit>,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage> {
        let Some(user_id) = filter_user_id(params) else {
            anyhow::bail!("local search requires a user_id filter");
        };
        let embeddings = self.embeddings.read().expect("local search lock poisoned");
        let mut hits = embeddings
            .iter()
            .filter_map(|(object_id, embedding)| {
                let (hit_user_id, _) = parse_fields(object_id).ok()?;
                if user_id != hit_user_id {
                    return None;
                }
                Some(SearchHit {
                    object_id: object_id.clone(),
                    score: cosine(embedding_query, embedding),
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(params.limit as usize);
        Ok(SearchResultPage {
            hits,
            next_page_token: None,
        })
    }

    async fn search_hybrid(
        &self,
        _text_query: &str,
        embedding_query: &Embedding<f32, Unit>,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage> {
        self.search_embedding(embedding_query, params).await
    }
}

#[async_trait::async_trait]
impl VectorStore<Embedding<f32, Unit>> for LocalSearch {
    async fn upsert_object_embedding(
        &self,
        object: &dyn DataObject,
        embedding: &Embedding<f32, Unit>,
    ) -> anyhow::Result<VectorUpsertResult> {
        let id = object.data_object_id();
        let dims = embedding.len();
        self.embeddings
            .write()
            .expect("local search lock poisoned")
            .insert(id.clone(), embedding.clone());
        Ok(VectorUpsertResult {
            id,
            fq_id: None,
            dims,
        })
    }

    async fn fetch_object_embedding(
        &self,
        object_id: &str,
    ) -> anyhow::Result<Option<Embedding<f32, Unit>>> {
        Ok(self
            .embeddings
            .read()
            .expect("local search lock poisoned")
            .get(object_id)
            .cloned())
    }
}

fn filter_user_id(params: &QueryParams) -> Option<i32> {
    params
        .base_filter
        .get("user_id")
        .and_then(|value| value.get("$eq"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
}

fn cosine(a: &Embedding<f32, Unit>, b: &Embedding<f32, Unit>) -> f64 {
    a.as_slice()
        .iter()
        .zip(b.as_slice())
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum()
}

fn embed_text(text: &str) -> anyhow::Result<Embedding<f32, Unit>> {
    let mut values = vec![0.0; DIMS];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
    {
        let hash = blake3::hash(token.as_bytes());
        let index = u32::from_le_bytes(hash.as_bytes()[..4].try_into().expect("four bytes"))
            as usize
            % DIMS;
        values[index] += 1.0;
    }
    if values.iter().all(|value| *value == 0.0) {
        values[0] = 1.0;
    }
    Embedding::from_vec_normalizing(values)
}

fn extract_text(value: &Value) -> String {
    let mut text = String::new();
    collect_text(value, &mut text);
    text
}

fn collect_text(value: &Value, text: &mut String) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "inlineData" || key == "data" {
                    continue;
                }
                collect_text(value, text);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_text(value, text);
            }
        }
        Value::String(value) if value.len() <= 4096 => {
            text.push_str(value);
            text.push('\n');
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_search_round_trips_and_filters_by_user() {
        let search = LocalSearch::default();
        let object = LocalObject {
            id: "u1-c7".to_string(),
        };
        let embedding = embed_text("napoleon").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            search
                .upsert_object_embedding(&object, &embedding)
                .await
                .unwrap();
            let page = search
                .search_embedding(
                    &embedding,
                    &QueryParams {
                        base_filter: serde_json::json!({"user_id": {"$eq": "1"}})
                            .as_object()
                            .unwrap()
                            .clone(),
                        limit: 10,
                        page_token: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(page.hits.len(), 1);
            assert_eq!(page.hits[0].object_id, "u1-c7");

            let other_user = search
                .search_embedding(
                    &embedding,
                    &QueryParams {
                        base_filter: serde_json::json!({"user_id": {"$eq": "2"}})
                            .as_object()
                            .unwrap()
                            .clone(),
                        limit: 10,
                        page_token: None,
                    },
                )
                .await
                .unwrap();
            assert!(other_user.hits.is_empty());

            let unfiltered = search
                .search_embedding(
                    &embedding,
                    &QueryParams {
                        base_filter: serde_json::Map::new(),
                        limit: 10,
                        page_token: None,
                    },
                )
                .await;
            assert!(unfiltered.is_err());
        });
    }

    struct LocalObject {
        id: String,
    }

    impl DataObject for LocalObject {
        fn data_object_id(&self) -> String {
            self.id.clone()
        }

        fn data_object_json(&self) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
            Ok(serde_json::Map::new())
        }
    }
}
