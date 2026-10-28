use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Full inference record for one attempt, including request params and provider
/// metadata. Admin-only: provider metadata may contain upstream response details.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InferenceMetaInfo {
    pub id: i32,
    pub task_run_id: Option<String>,
    pub attempt: Option<i32>,
    pub task_type: Option<String>,
    pub provider: String,
    pub backend: String,
    pub model: String,
    pub duration_ms: i64,
    pub tokens_in: Option<i64>,
    pub tokens_in_cached: Option<i64>,
    pub tokens_out: Option<i64>,
    pub tokens_thought: Option<i64>,
    pub params: Value,
    pub provider_metadata: Value,
    pub created_at: DateTime<Utc>,
}

impl From<crate::model::inference_meta::Model> for InferenceMetaInfo {
    fn from(row: crate::model::inference_meta::Model) -> Self {
        Self {
            id: row.id,
            task_run_id: row.task_run_id,
            attempt: row.attempt,
            task_type: row.task_type,
            provider: row.provider,
            backend: row.backend,
            model: row.model,
            duration_ms: row.duration_ms,
            tokens_in: row.tokens_in,
            tokens_in_cached: row.tokens_in_cached,
            tokens_out: row.tokens_out,
            tokens_thought: row.tokens_thought,
            params: row.params,
            provider_metadata: row.provider_metadata,
            created_at: row.created_at,
        }
    }
}
