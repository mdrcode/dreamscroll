use anyhow::anyhow;
use axum::{body, http::StatusCode};
use axum_extra::extract::Multipart;

use crate::{api, auth};

pub async fn insert_capture_from_multipart(
    user_api: &api::UserApiClient,
    context: &auth::Context,
    multipart: Multipart,
    max_upload_bytes: usize,
) -> Result<api::CaptureInfo, api::ApiError> {
    let image_payload = match extract_first_image_payload(multipart).await {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            return Err(api::ApiError::bad_request(anyhow!(
                "No valid image data found."
            )));
        }
        // The body limit trips while the stream is read, so it surfaces here.
        Err(error) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return Err(api::ApiError::upload_too_large(max_upload_bytes, None));
        }
        Err(error) => return Err(api::ApiError::bad_request(anyhow!(error))),
    };

    let image_bytes = image_payload.bytes.len();

    // Guards the image itself should the body limit ever exceed it.
    if image_bytes > max_upload_bytes {
        return Err(api::ApiError::upload_too_large(max_upload_bytes, Some(image_bytes as u64)));
    }

    user_api
        .insert_capture(context, image_payload.bytes)
        .await
}

#[derive(Debug)]
struct ImagePayload {
    bytes: body::Bytes,
}

async fn extract_first_image_payload(
    mut mp: Multipart,
) -> Result<Option<ImagePayload>, axum_extra::extract::multipart::MultipartError> {
    loop {
        let f = match mp.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => return Err(e),
        };

        let is_image_content = f
            .content_type()
            .map(|ct| ct.starts_with("image/"))
            .unwrap_or(false);
        if !is_image_content {
            continue;
        }

        match f.bytes().await {
            Ok(bytes) => return Ok(Some(ImagePayload { bytes })),
            Err(e) => return Err(e),
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
    async fn extract_first_image_payload_picks_the_first_image() {
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

    /// The upload path branches on this to answer 413 rather than 400.
    #[tokio::test]
    async fn body_over_the_limit_reports_payload_too_large() {
        let boundary = "BOUNDARY_BIG";
        let big = "x".repeat(3 * 1024 * 1024);
        let body = multipart_body(
            boundary,
            &[&format!(
                "Content-Disposition: form-data; name=\"image\"; filename=\"big.png\"\r\nContent-Type: image/png\r\n\r\n{big}"
            )],
        );

        let mp = make_multipart(body, boundary).await;
        let error = extract_first_image_payload(mp)
            .await
            .expect_err("body over the extractor limit should fail");

        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// The body is opaque by design, so the sizes have to live in the message.
    #[test]
    fn too_large_names_the_actual_and_maximum_sizes() {
        let error = api::ApiError::upload_too_large(12 * 1024 * 1024, Some(13 * 1024 * 1024));

        assert_eq!(error.http_status_code, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(error.message().contains("13.0 MB"), "{}", error.message());
        assert!(error.message().contains("12.0 MB"), "{}", error.message());
    }
}
