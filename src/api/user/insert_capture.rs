use anyhow::anyhow;
use bytes::Bytes;
use chrono::Utc;
use sea_orm::{TryIntoModel, prelude::*};

use crate::{api::*, auth, database::DbHandle, model, storage};

pub async fn insert_capture(
    db: &DbHandle,
    storage: &dyn storage::StorageProvider,
    user_context: &auth::Context,
    bytes: Bytes,
    dedupe: bool,
    created_at: Option<chrono::DateTime<Utc>>,
) -> Result<model::capture::ModelEx, ApiError> {
    let user_id = user_context.user_id();
    let shard = user_context.storage_shard();

    let media_type = supported_image(&bytes)?;
    tracing::debug!("Media type inferred as {}", media_type.mime_type());

    // Optionally prevent duplicate imports as a convenience. Scoped to the
    // user: without the filter, a 409 would tell one user that another user
    // already holds the exact image they're importing.
    let hash_blake3 = blake3::hash(&bytes);
    if dedupe
        && model::media::Entity::find()
            .filter(model::media::Column::UserId.eq(user_id))
            .filter(model::media::Column::HashBlake3.eq(hash_blake3.to_hex().to_string()))
            .one(&db.conn)
            .await?
            .is_some()
    {
        return Err(ApiError::conflict(anyhow!(
            "An image with the same blake3 content hash already exists for this account."
        )));
    }

    let bytes_len = bytes.len();
    let handle = storage
        .store_bytes(
            bytes,
            shard,
            Some(media_type.extension()),
            Some(media_type.mime_type()),
        )
        .await?;

    let media_builder = model::media::ActiveModel::builder()
        .set_user_id(user_id)
        .set_bytes(bytes_len as i64)
        .set_mime_type(Some(media_type.mime_type().to_string()))
        .set_hash_blake3(Some(hash_blake3.to_hex().to_string()))
        .set_storage_provider(handle.provider)
        .set_storage_bucket(handle.bucket)
        .set_storage_user_shard(handle.user_shard)
        .set_storage_uuid(handle.uuid)
        .set_storage_extension(handle.extension);

    let created_at = created_at.unwrap_or_else(Utc::now);

    let capture = model::capture::ActiveModel::builder()
        .set_user_id(user_context.user_id())
        .set_created_at(created_at)
        .add_media(media_builder)
        .save(&db.conn)
        .await?;

    let model = capture.try_into_model()?;

    Ok(model)
}

/// Refuse HEIC/HEIF (`infer` calls it `image/heif`), which only Safari renders,
/// and anything we cannot identify. A blocklist, not a whitelist: the accepted
/// set grows, and enumerating it would reject new standard formats.
fn supported_image(bytes: &[u8]) -> Result<infer::Type, ApiError> {
    let unidentified = || {
        ApiError::unsupported_media_type(anyhow!(
            "Could not identify the uploaded file format ({} bytes)",
            bytes.len()
        ))
    };

    let media_type = infer::get(bytes).ok_or_else(unidentified)?;

    if !media_type.mime_type().starts_with("image/") {
        return Err(ApiError::bad_request(anyhow!(
            "Uploaded media is not an image."
        )));
    }

    if matches!(media_type.mime_type(), "image/heic" | "image/heif") {
        return Err(ApiError::unsupported_media_type(anyhow!(
            "Refused {} bytes of {}: no browser outside Safari renders it",
            bytes.len(),
            media_type.mime_type()
        )));
    }

    Ok(media_type)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    /// An ISO-BMFF `ftyp` box, the shape an iPhone writes.
    fn heic_header(brand: &[u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x18]);
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(brand);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"mif1heic");
        bytes.resize(64, 0);
        bytes
    }

    #[test]
    fn refuses_heic_because_no_other_browser_renders_it() {
        let error = supported_image(&heic_header(b"heic")).expect_err("HEIC must be refused");

        assert_eq!(error.http_status_code, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn accepts_standard_formats() {
        let jpeg = b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00";
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";

        assert_eq!(supported_image(jpeg).unwrap().mime_type(), "image/jpeg");
        assert_eq!(supported_image(png).unwrap().mime_type(), "image/png");
    }

    /// Unrecognised bytes used to fall through to a 500.
    #[test]
    fn unidentifiable_bytes_are_415_not_500() {
        let error = supported_image(b"definitely not an image").expect_err("unknown bytes");

        assert_eq!(error.http_status_code, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}
