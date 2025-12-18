use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    body,
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Set};
use uuid::Uuid;

use crate::entity::{capture, media};
use crate::webui::{WebState, prelude::*};

pub async fn upload(
    State(state): State<Arc<WebState>>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let media_bytes = match extract_bytes(multipart, "image").await? {
        Some(bytes) => bytes,
        None => {
            return Err(AppError::bad_request(anyhow!(
                "No image data found in upload."
            )));
        }
    };

    // Limit to 5MB TODO currently this is already limited by axum body limit layer
    if media_bytes.len() > 5 * 1024 * 1024 {
        return Err(AppError::payload_too_large(anyhow!(
            "File size exceeds limit."
        )));
    }

    // Generate unique filename
    let media_uuid = Uuid::new_v4().to_string();
    let filename = format!("{}.jpg", media_uuid);
    let upload_dir = state.facility.local_media_path();
    let upload_path = Path::new(&upload_dir).join(&filename);

    // Write the file to persistent storage
    tokio::fs::write(&upload_path, &media_bytes).await?;

    // Insert new capture record into the database
    let new_capture = capture::ActiveModel {
        uuid: Set(media_uuid.clone()),
        created_at: Set(Utc::now()),
        ..Default::default()
    };

    let capture_result = new_capture.insert(&state.db.conn).await?;

    // Insert new media record linked to the capture
    let new_media = media::ActiveModel {
        filename: Set(filename),
        capture_id: Set(Some(capture_result.id)),
        ..Default::default()
    };

    new_media.insert(&state.db.conn).await?;

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
