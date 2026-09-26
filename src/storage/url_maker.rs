use crate::storage::StorageHandle;

use crate::config;
use google_cloud_auth::{credentials, signer::Signer};
use google_cloud_storage::{builder::storage::SignedUrlBuilder, http::Method};
use std::time::Duration;

#[derive(Clone)]
pub struct UrlMaker {
    // "/media" if full URL is something like http://localhost:8000/media/foo.jpg
    local_url_prefix: Option<String>,
    // e.g. "http://localhost:4443 if running fake-gcs via Docker
    gcloud_emulator_endpoint: Option<String>,
    gcloud_signer: Option<Signer>,
}

impl UrlMaker {
    pub fn from_config(cfg: &config::Config) -> anyhow::Result<Self> {
        let gcloud_signer = if cfg.storage_backend == config::StorageBackend::GCloud
            && cfg.storage_gcloud_emulator.is_none()
        {
            tracing::info!("Initializing GCS signed URL signer");
            Some(credentials::Builder::default().build_signer()?)
        } else {
            None
        };

        Ok(Self {
            local_url_prefix: cfg.storage_local_url_prefix.clone(),
            gcloud_emulator_endpoint: cfg.storage_gcloud_emulator.clone(),
            gcloud_signer,
        })
    }
}

impl UrlMaker {
    pub async fn make_url(&self, id: &StorageHandle) -> anyhow::Result<String> {
        match id.provider.as_str() {
            "local" => Ok(self.make_local_url(id)),
            "gcloud" => {
                if self.gcloud_emulator_endpoint.is_some() {
                    Ok(self.make_emulator_url(id))
                } else {
                    self.make_signed_gcloud_url(id, Duration::from_secs(7 * 24 * 60 * 60))
                        .await
                }
            }
            other => anyhow::bail!("Unknown storage provider: {other}"),
        }
    }

    pub fn make_local_url(&self, id: &StorageHandle) -> String {
        if self.local_url_prefix.is_none() {
            tracing::error!("Asked to make local URL but local URL prefix is not configured");
            unimplemented!("Local URL prefix is not configured");
        }

        format!(
            "{}/{}/{}{}",
            self.local_url_prefix.as_ref().unwrap(),
            id.user_shard,
            id.uuid,
            id.extension
                .as_ref()
                .map(|ext| format!(".{}", ext))
                .unwrap_or_default()
        )
    }

    fn make_emulator_url(&self, id: &StorageHandle) -> String {
        format!(
            "{}/storage/v1/b/{}/o/{}%2F{}{}?alt=media",
            self.gcloud_emulator_endpoint
                .as_deref()
                .unwrap_or("http://localhost:4443"),
            id.bucket.as_ref().unwrap(),
            id.user_shard,
            id.uuid,
            id.extension
                .as_ref()
                .map(|ext| format!(".{}", ext))
                .unwrap_or_default()
        )
    }

    /// Creates a V4-signed GET URL for a production GCS object.
    ///
    /// The caller must perform application-level ownership checks before calling
    /// this method. The returned URL is a bearer credential and must not be logged.
    pub async fn make_signed_gcloud_url(
        &self,
        id: &StorageHandle,
        expiration: Duration,
    ) -> anyhow::Result<String> {
        if self.gcloud_emulator_endpoint.is_some() {
            return Ok(self.make_emulator_url(id));
        }

        let signer = self
            .gcloud_signer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GCS URL signer is not configured"))?;
        let bucket = id
            .bucket
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Cannot sign GCS URL without a bucket"))?;
        let object = format!(
            "{}/{}",
            id.user_shard,
            id.uuid.to_string()
                + &id
                    .extension
                    .as_ref()
                    .map(|ext| format!(".{ext}"))
                    .unwrap_or_default()
        );

        SignedUrlBuilder::for_object(format!("projects/_/buckets/{bucket}"), object)
            .with_method(Method::GET)
            .with_expiration(expiration)
            .sign_with(signer)
            .await
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn gcloud_handle() -> StorageHandle {
        StorageHandle {
            provider: "gcloud".to_string(),
            bucket: Some("test-bucket".to_string()),
            user_shard: "test-shard".to_string(),
            uuid: Uuid::nil(),
            extension: Some("png".to_string()),
        }
    }

    fn local_url_maker() -> UrlMaker {
        UrlMaker {
            local_url_prefix: Some("http://localhost:8000/media".to_string()),
            gcloud_emulator_endpoint: None,
            gcloud_signer: None,
        }
    }

    fn local_handle(extension: Option<&str>) -> StorageHandle {
        StorageHandle {
            provider: "local".to_string(),
            bucket: None,
            user_shard: "test-shard".to_string(),
            uuid: Uuid::nil(),
            extension: extension.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn local_url_includes_extension() {
        let url = local_url_maker()
            .make_url(&local_handle(Some("png")))
            .await
            .expect("local URL should be created");

        assert_eq!(
            url,
            "http://localhost:8000/media/test-shard/00000000-0000-0000-0000-000000000000.png"
        );
    }

    #[tokio::test]
    async fn local_url_supports_objects_without_extension() {
        let url = local_url_maker()
            .make_url(&local_handle(None))
            .await
            .expect("local URL should be created");

        assert_eq!(
            url,
            "http://localhost:8000/media/test-shard/00000000-0000-0000-0000-000000000000"
        );
    }

    #[tokio::test]
    async fn emulator_url_uses_configured_endpoint() {
        let url_maker = UrlMaker {
            local_url_prefix: None,
            gcloud_emulator_endpoint: Some("http://fake-gcs:4443".to_string()),
            gcloud_signer: None,
        };

        let url = url_maker
            .make_url(&gcloud_handle())
            .await
            .expect("emulator URL should be created");

        assert_eq!(
            url,
            "http://fake-gcs:4443/storage/v1/b/test-bucket/o/test-shard%2F00000000-0000-0000-0000-000000000000.png?alt=media"
        );
    }

    #[tokio::test]
    async fn production_url_requires_signer() {
        let url_maker = UrlMaker {
            local_url_prefix: None,
            gcloud_emulator_endpoint: None,
            gcloud_signer: None,
        };

        let error = url_maker
            .make_url(&gcloud_handle())
            .await
            .expect_err("production URL should require a signer");

        assert_eq!(error.to_string(), "GCS URL signer is not configured");
    }

    #[tokio::test]
    async fn unknown_provider_returns_error() {
        let url_maker = local_url_maker();
        let mut handle = local_handle(None);
        handle.provider = "unknown".to_string();

        let error = url_maker
            .make_url(&handle)
            .await
            .expect_err("unknown provider should fail");

        assert_eq!(error.to_string(), "Unknown storage provider: unknown");
    }

    #[tokio::test]
    async fn signed_url_method_preserves_emulator_behavior() {
        let url_maker = UrlMaker {
            local_url_prefix: None,
            gcloud_emulator_endpoint: Some("http://fake-gcs:4443".to_string()),
            gcloud_signer: None,
        };

        let url = url_maker
            .make_signed_gcloud_url(&gcloud_handle(), Duration::from_secs(60))
            .await
            .expect("emulator URL should be created");

        assert!(url.starts_with("http://fake-gcs:4443/storage/v1/"));
        assert!(!url.contains("X-Goog-Signature"));
    }
}
