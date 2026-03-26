use anyhow::anyhow;
use axum::body;
use axum_extra::extract::Multipart;

use crate::{api, auth};

const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

pub async fn insert_capture_from_multipart(
    user_api: &api::UserApiClient,
    context: &auth::Context,
    multipart: Multipart,
) -> Result<api::CaptureInfo, api::ApiError> {
    let image_payload = match extract_first_image_payload(multipart).await? {
        Some(payload) => payload,
        None => {
            return Err(api::ApiError::bad_request(anyhow!(
                "No valid image data found."
            )));
        }
    };

    if let Some(filename) = image_payload.original_filename.as_deref() {
        tracing::info!(original_filename = %filename, "Upload multipart filename detected");
    }

    if image_payload.bytes.len() > MAX_UPLOAD_BYTES {
        return Err(api::ApiError::payload_too_large(anyhow!(
            "Payload too large."
        )));
    }

    let capture = user_api
        .insert_capture(context, image_payload.bytes)
        .await?;
    Ok(capture)
}

struct ImagePayload {
    bytes: body::Bytes,
    original_filename: Option<String>,
}

async fn extract_first_image_payload(
    mut mp: Multipart,
) -> Result<Option<ImagePayload>, api::ApiError> {
    loop {
        let f = match mp.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                return Err(api::ApiError::bad_request(anyhow!(e)));
            }
        };

        let is_image_content = f
            .content_type()
            .map(|ct| ct.starts_with("image/"))
            .unwrap_or(false);
        let original_filename = f.file_name().map(ToString::to_string);

        if !is_image_content {
            continue;
        }

        match f.bytes().await {
            Ok(bytes) => {
                return Ok(Some(ImagePayload {
                    bytes,
                    original_filename,
                }));
            }
            Err(e) => {
                return Err(api::ApiError::bad_request(anyhow!(e)));
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::FromRequest,
        http::{Request, header::CONTENT_TYPE},
    };

    fn multipart_body(boundary: &str, parts: &[&str]) -> Vec<u8> {
        let mut body = String::new();
        for p in parts {
            body.push_str("--");
            body.push_str(boundary);
            body.push_str("\r\n");
            body.push_str(p);
            body.push_str("\r\n");
        }
        body.push_str("--");
        body.push_str(boundary);
        body.push_str("--\r\n");
        body.into_bytes()
    }

    async fn make_multipart(body: Vec<u8>, boundary: &str) -> Multipart {
        let req = Request::builder()
            .header(
                CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body))
            .expect("failed to build request");

        Multipart::from_request(req, &())
            .await
            .expect("failed to create Multipart extractor")
    }

    #[tokio::test]
    async fn extract_first_image_payload_picks_first_image_and_filename() {
        let boundary = "BOUNDARY123";
        let body = multipart_body(
            boundary,
            &[
                "Content-Disposition: form-data; name=\"note\"\r\n\r\nhello",
                "Content-Disposition: form-data; name=\"img1\"; filename=\"first.png\"\r\nContent-Type: image/png\r\n\r\nPNG1",
                "Content-Disposition: form-data; name=\"img2\"; filename=\"second.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nJPG2",
            ],
        );

        let mp = make_multipart(body, boundary).await;
        let payload = extract_first_image_payload(mp)
            .await
            .expect("extract should succeed")
            .expect("first image should exist");

        assert_eq!(payload.bytes, body::Bytes::from_static(b"PNG1"));
        assert_eq!(payload.original_filename.as_deref(), Some("first.png"));
    }

    #[tokio::test]
    async fn extract_first_image_payload_ignores_non_image_parts() {
        let boundary = "BOUNDARY456";
        let body = multipart_body(
            boundary,
            &[
                "Content-Disposition: form-data; name=\"file\"; filename=\"doc.txt\"\r\nContent-Type: text/plain\r\n\r\nnot-image",
                "Content-Disposition: form-data; name=\"img\"; filename=\"photo.webp\"\r\nContent-Type: image/webp\r\n\r\nWEBP",
            ],
        );

        let mp = make_multipart(body, boundary).await;
        let payload = extract_first_image_payload(mp)
            .await
            .expect("extract should succeed")
            .expect("image should exist");

        assert_eq!(payload.bytes, body::Bytes::from_static(b"WEBP"));
        assert_eq!(payload.original_filename.as_deref(), Some("photo.webp"));
    }

    #[tokio::test]
    async fn extract_first_image_payload_returns_none_when_no_images() {
        let boundary = "BOUNDARY789";
        let body = multipart_body(
            boundary,
            &[
                "Content-Disposition: form-data; name=\"a\"\r\n\r\nhello",
                "Content-Disposition: form-data; name=\"b\"; filename=\"x.txt\"\r\nContent-Type: text/plain\r\n\r\nworld",
            ],
        );

        let mp = make_multipart(body, boundary).await;
        let payload = extract_first_image_payload(mp)
            .await
            .expect("extract should succeed");

        assert!(payload.is_none());
    }
}
