use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::api::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillType {
    SearchIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillRequest {
    pub backfill_type: BackfillType,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub force_all: bool,
    pub limit: Option<u64>,
    pub capture_ids: Option<Vec<i32>>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillResponse {
    pub backfill_type: BackfillType,
    pub mode: String,
    pub candidate_count: usize,
    pub enqueued_count: usize,
    pub skipped_count: usize,
    pub skipped_ids: Vec<i32>,
}

pub async fn enqueue(
    service_api: &ServiceApiClient,
    beacon: &crate::task::Beacon,
    req: BackfillRequest,
) -> Result<BackfillResponse, ApiError> {
    let has_ids = req
        .capture_ids
        .as_ref()
        .map(|ids| !ids.is_empty())
        .unwrap_or(false);

    if req.all == has_ids {
        return Err(ApiError::bad_request(anyhow!(
            "Provide either --all or explicit capture_ids, but not both"
        )));
    }

    if req.force_all && !req.all {
        return Err(ApiError::bad_request(anyhow!(
            "force_all requires all=true"
        )));
    }

    if req.all && !req.force_all && req.limit.is_none() {
        return Err(ApiError::bad_request(anyhow!(
            "all=true requires either limit or force_all=true"
        )));
    }

    let backfill_type = req.backfill_type;
    match backfill_type {
        BackfillType::SearchIndex => {
            let mode;
            let candidate_ids = if req.all {
                mode = "all".to_string();
                let limit = if req.force_all { None } else { req.limit };
                service_api.get_captures_need_search_index(limit).await?
            } else {
                mode = "ids".to_string();
                req.capture_ids.unwrap_or_default()
            };

            let candidate_count = candidate_ids.len();

            if req.dry_run {
                return Ok(BackfillResponse {
                    backfill_type,
                    mode,
                    candidate_count,
                    enqueued_count: 0,
                    skipped_count: 0,
                    skipped_ids: Vec::new(),
                });
            }

            let mut enqueued_count = 0usize;
            let mut skipped_ids = Vec::new();

            for capture_id in candidate_ids {
                match beacon.signal_search_index(capture_id).await {
                    Ok(()) => {
                        enqueued_count += 1;
                    }
                    Err(err) => {
                        tracing::warn!(capture_id, error = ?err, "Failed enqueue in admin backfill");
                        skipped_ids.push(capture_id);
                    }
                }
            }

            Ok(BackfillResponse {
                backfill_type,
                mode,
                candidate_count,
                enqueued_count,
                skipped_count: skipped_ids.len(),
                skipped_ids,
            })
        }
    }
}
