use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    body,
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use axum_login::{AuthSession, AuthUser};

use crate::{api, auth};

use super::WebState;

#[tracing::instrument(skip(auth, state, multipart))]
pub async fn upload(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    multipart: Multipart,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    tracing::info!("Processing upload for user ID {}", user.id());

    let media_bytes = match extract_bytes(multipart, "image").await? {
        Some(bytes) => bytes,
        None => {
            return Err(api::ApiError::bad_request(anyhow!("No image data found.")));
        }
    };

    // Limit to 5MB TODO currently this is already limited by axum body limit layer
    if media_bytes.len() > 5 * 1024 * 1024 {
        return Err(api::ApiError::payload_too_large(anyhow!(
            "Payload too large."
        )));
    }

    let handle = state
        .storage
        .store_bytes(&media_bytes, user.storage_shard())
        .await?;
    tracing::info!("Media stored for u {} handle: {:?}", user.id(), &handle);

    let cap = state.user_api.insert_capture(&user.into(), handle).await?;
    tracing::info!("Capture {} inserted via upload", cap.id);

    // Redirect to home page to show the timeline
    Ok(Redirect::to("/").into_response())
}

async fn extract_bytes(mut mp: Multipart, field: &str) -> anyhow::Result<Option<body::Bytes>> {
    while let Ok(Some(f)) = mp.next_field().await {
        if f.name().unwrap_or("") != field {
            continue;
        }

        match f.bytes().await {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) => return Err(e.into()),
        };
    }

    Ok(None)
}
