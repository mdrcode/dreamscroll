use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    Json, body,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::Multipart;
use chrono::{DateTime, Utc};

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

/// POST /api/captures/import - Import a capture with explicit creation
/// timestamp and fail on duplicate image hash.
///
/// Multipart fields:
/// - `image` (required): media bytes for the capture
/// - `created_at` (required): RFC3339 timestamp for the capture creation time
///
/// Requires JWT authentication.
pub async fn post(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    multipart: Multipart,
) -> Result<Response, api::ApiError> {
    // The body limit enforces `MAX_UPLOAD_BYTES`; this only explains it.
    let import_form = extract_import_form(multipart, state.max_upload_bytes).await?;

    let result = state
        .user_api
        .import_capture(
            &user.into(),
            import_form.media_bytes,
            import_form.created_at,
        )
        .await;

    match result {
        Ok(cap) => Ok(Json(cap).into_response()),
        Err(e) => {
            if e.http_status_code == StatusCode::CONFLICT {
                Ok(StatusCode::CONFLICT.into_response())
            } else {
                tracing::error!(error = ?e, "Import capture failed");
                Err(e)
            }
        }
    }
}

struct ImportForm {
    media_bytes: body::Bytes,
    created_at: DateTime<Utc>,
}

/// A body over `MAX_UPLOAD_BYTES` is a 413 that names the limit; anything else
/// is a malformed request.
fn multipart_error(
    error: axum_extra::extract::multipart::MultipartError,
    max_upload_bytes: usize,
) -> api::ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return api::ApiError::upload_too_large(max_upload_bytes, None);
    }
    api::ApiError::bad_request(anyhow!(error))
}

async fn extract_import_form(
    mut mp: Multipart,
    max_upload_bytes: usize,
) -> Result<ImportForm, api::ApiError> {
    let mut media_bytes: Option<body::Bytes> = None;
    let mut created_at: Option<DateTime<Utc>> = None;

    loop {
        let field = match mp.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            // The old `while let Ok(..)` swallowed this, reporting "No image data found."
            Err(error) => return Err(multipart_error(error, max_upload_bytes)),
        };

        match field.name().unwrap_or("") {
            "image" => {
                media_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|error| multipart_error(error, max_upload_bytes))?,
                );
            }
            "created_at" => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| multipart_error(error, max_upload_bytes))?;
                let parsed = DateTime::parse_from_rfc3339(&value).map_err(|e| {
                    api::ApiError::bad_request(anyhow!("Invalid created_at '{}': {e}", value))
                })?;
                created_at = Some(parsed.with_timezone(&Utc));
            }
            _ => {}
        }
    }

    let media_bytes =
        media_bytes.ok_or_else(|| api::ApiError::bad_request(anyhow!("No image data found.")))?;
    let created_at = created_at
        .ok_or_else(|| api::ApiError::bad_request(anyhow!("Missing created_at field.")))?;

    Ok(ImportForm {
        media_bytes,
        created_at,
    })
}
