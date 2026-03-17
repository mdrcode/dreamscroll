use anyhow::anyhow;
use axum::body;
use axum_extra::extract::Multipart;

use crate::{api, auth};

const IMAGE_FIELD_NAME: &str = "image";
const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

pub async fn insert_capture_from_multipart(
    user_api: &api::UserApiClient,
    context: &auth::Context,
    multipart: Multipart,
) -> Result<api::CaptureInfo, api::ApiError> {
    let media_bytes = match extract_bytes(multipart, IMAGE_FIELD_NAME).await? {
        Some(bytes) => bytes,
        None => {
            return Err(api::ApiError::bad_request(anyhow!("No image data found.")));
        }
    };

    if media_bytes.len() > MAX_UPLOAD_BYTES {
        return Err(api::ApiError::payload_too_large(anyhow!("Payload too large.")));
    }

    let capture = user_api.insert_capture(context, media_bytes).await?;
    Ok(capture)
}

async fn extract_bytes(
    mut mp: Multipart,
    field: &str,
) -> Result<Option<body::Bytes>, api::ApiError> {
    while let Ok(Some(f)) = mp.next_field().await {
        if f.name().unwrap_or("") != field {
            continue;
        }

        match f.bytes().await {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) => return Err(api::ApiError::bad_request(anyhow!(e))),
        };
    }

    Ok(None)
}
