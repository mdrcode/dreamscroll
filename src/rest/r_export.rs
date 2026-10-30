use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, header},
    response::{IntoResponse, Response},
};
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

const NDJSON: &str = "application/x-ndjson";
/// Cursor for the next page. A header rather than a line so the body stays pure
/// JSONL; absent when the export is exhausted.
const NEXT_AFTER_ID: &str = "x-next-after-id";

#[derive(Debug, Default, Deserialize)]
pub struct ExportQuery {
    /// Restrict the export to one user; absent means every user in one stream.
    #[serde(default)]
    user_id: Option<i32>,
    /// Exclusive cursor, taken from the previous response's `x-next-after-id`.
    #[serde(default)]
    after_id: i32,
    #[serde(default)]
    limit: Option<u64>,
}

/// GET /api/admin/export/{kind} - JSONL export, one `ExportRow` per line.
///
/// Kinds:
/// - `entities` — knowledge entities, paged by `after_id`, `?user_id=` optional.
/// - `users` — the user list, which the export tooling uses to discover ids.
///
/// Admin only. Responses are `application/x-ndjson`; when more rows exist the
/// page carries `x-next-after-id`.
///
/// Requires JWT authentication for an admin user.
pub async fn get(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(kind): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, api::ApiError> {
    match kind.as_str() {
        "entities" => {
            let req = api::ExportEntitiesRequest {
                user_id: query.user_id,
                after_id: query.after_id,
                limit: query.limit.unwrap_or(api::EXPORT_DEFAULT_LIMIT),
            };
            let (rows, next_after_id) = state.admin_api.export_entities(&user.into(), req).await?;

            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(NDJSON));
            if let Some(next_after_id) = next_after_id {
                headers.insert(
                    HeaderName::from_static(NEXT_AFTER_ID),
                    HeaderValue::from_str(&next_after_id.to_string())
                        .map_err(|err| api::ApiError::internal(anyhow!(err)))?,
                );
            }

            Ok((headers, ndjson(&rows)?).into_response())
        }
        "users" => {
            let rows = state
                .admin_api
                .export_users(&user.into(), query.user_id)
                .await?;

            Ok(([(header::CONTENT_TYPE, NDJSON)], ndjson(&rows)?).into_response())
        }
        other => Err(api::ApiError::not_found(anyhow!(
            "Unknown export '{other}'; expected 'entities' or 'users'"
        ))),
    }
}

fn ndjson<T: Serialize>(rows: &[T]) -> Result<String, api::ApiError> {
    let mut body = String::new();
    for row in rows {
        body.push_str(&serde_json::to_string(row).map_err(api::ApiError::internal)?);
        body.push('\n');
    }
    Ok(body)
}
