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
    let import_form = extract_import_form(multipart)
        .await
        .map_err(api::ApiError::bad_request)?;

    // Limit to 5MB. TODO currently this is already limited by axum body limit layer?
    if import_form.media_bytes.len() > 5 * 1024 * 1024 {
        return Err(api::ApiError::payload_too_large(anyhow!(
            "Payload too large."
        )));
    }

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
            if e.status_code == StatusCode::CONFLICT {
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

async fn extract_import_form(mut mp: Multipart) -> anyhow::Result<ImportForm> {
    let mut media_bytes: Option<body::Bytes> = None;
    let mut created_at: Option<DateTime<Utc>> = None;

    while let Ok(Some(field)) = mp.next_field().await {
        match field.name().unwrap_or("") {
            "image" => {
                media_bytes = Some(field.bytes().await?);
            }
            "created_at" => {
                let value = field.text().await?;
                let parsed = DateTime::parse_from_rfc3339(&value)
                    .map_err(|e| anyhow!("Invalid created_at '{}': {e}", value))?
                    .with_timezone(&Utc);
                created_at = Some(parsed);
            }
            _ => {}
        }
    }

    let media_bytes = media_bytes.ok_or_else(|| anyhow!("No image data found."))?;
    let created_at = created_at.ok_or_else(|| anyhow!("Missing created_at field."))?;

    Ok(ImportForm {
        media_bytes,
        created_at,
    })
}
