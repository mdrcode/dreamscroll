use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    body,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Set};
use uuid::Uuid;

use crate::entity::{capture, media};
use crate::webui::WebState;

pub async fn upload(State(state): State<Arc<WebState>>, multipart: Multipart) -> Response {
    let (media_bytes, _) = match extract(multipart).await {
        Ok(result) => result,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    if media_bytes.is_none() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let media_bytes = media_bytes.unwrap();

    // Limit to 10MB
    if media_bytes.len() > 10 * 1024 * 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }

    // Generate unique filename
    let media_uuid = Uuid::new_v4().to_string();
    let filename = format!("{}.jpg", media_uuid);
    let upload_dir = state.facility.local_media_path();
    let upload_path = Path::new(&upload_dir).join(&filename);

    // Write the file to persistent storage
    if tokio::fs::write(&upload_path, &media_bytes).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Insert new capture record into the database

    let new_capture = capture::ActiveModel {
        uuid: Set(media_uuid.clone()),
        created_at: Set(Utc::now()),
        ..Default::default()
    };

    let capture_result = match new_capture.insert(&state.db.conn).await {
        Ok(result) => result,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Insert new media record linked to the capture
    let new_media = media::ActiveModel {
        filename: Set(filename),
        capture_id: Set(Some(capture_result.id)),
        ..Default::default()
    };

    if new_media.insert(&state.db.conn).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Redirect to home page to show the timeline
    Redirect::to("/").into_response()
}

async fn extract(mut multipart: Multipart) -> anyhow::Result<(Option<body::Bytes>, String)> {
    let mut image_bytes = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("");

        if field_name == "image" {
            match field.bytes().await {
                Ok(bytes) => image_bytes = Some(bytes),
                Err(_) => return Err(anyhow!("Failed to read image bytes")),
            };
        }
    }

    Ok((image_bytes, "tuple dummy".to_string()))
}
