use axum::{
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use std::path::Path;
use uuid::Uuid;

pub async fn upload(mut multipart: Multipart) -> Response {
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name != "image" {
            continue;
        }

        let data = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };

        // Limit to 10MB
        if data.len() > 10 * 1024 * 1024 {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }

        // Generate unique filename
        let filename = format!("{}.jpg", Uuid::new_v4());
        let upload_dir = "localdev/uploads";
        let upload_path = Path::new(upload_dir).join(&filename);

        // Create uploads directory if it doesn't exist
        if tokio::fs::create_dir_all(upload_dir).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        // Write the file to persistent storage
        if tokio::fs::write(&upload_path, &data).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        // Redirect to home page to show the timeline
        return Redirect::to("/").into_response();
    }

    StatusCode::BAD_REQUEST.into_response()
}
