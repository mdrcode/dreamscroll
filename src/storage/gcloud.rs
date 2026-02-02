use std::path::PathBuf;

use async_trait::async_trait;
use axum::body::Bytes;
use google_cloud_storage::client::Storage;
use uuid::Uuid;

use super::*;

#[derive(Debug, Clone)]
pub struct GCloudConfig {
    /// Optional emulator endpoint (e.g., "http://localhost:4443")
    /// If None, uses production GCS
    pub emulator_endpoint: Option<String>,
    /// The GCS bucket name (without the projects/_/buckets/ prefix)
    pub bucket: String,
}

#[derive(Clone)]
pub struct GCloudStorageProvider {
    config: GCloudConfig,
    client: Storage,
    /// The bucket path in the format required by the API: "projects/_/buckets/{bucket}"
    bucket_path: String,
}

impl GCloudStorageProvider {
    pub async fn new(config: GCloudConfig) -> Self {
        let mut builder = Storage::builder();

        // If emulator endpoint is set, configure for emulator use
        if let Some(ref endpoint) = config.emulator_endpoint {
            builder = builder.with_endpoint(endpoint.clone());
        }

        let client = builder
            .build()
            .await
            .expect("Failed to create GCloud Storage client");

        let bucket_path = format!("projects/_/buckets/{}", config.bucket);

        Self {
            config,
            client,
            bucket_path,
        }
    }
}

#[async_trait]
impl provider::StorageProvider for GCloudStorageProvider {
    async fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageIdentity> {
        let uuid = Uuid::new_v4().to_string();
        let bytes_data = Bytes::copy_from_slice(data);

        self.client
            .write_object(&self.bucket_path, &uuid, bytes_data)
            .send_buffered()
            .await
            .map_err(|e| {
                tracing::error!("Failed to store object in GCS: {:?}", e);
                anyhow::anyhow!("Failed to store object in GCS: {}", e)
            })?;

        tracing::debug!("Stored object {} in bucket {}", uuid, self.config.bucket);
        Ok(StorageIdentity {
            storage_provider: "gcloud".to_string(),
            provider_id: uuid,
            provider_shard: None,
            provider_bucket: Some(self.config.bucket.clone()),
        })
    }

    async fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageIdentity> {
        let uuid = Uuid::new_v4().to_string();

        tracing::info!(
            "Storing from local path {:?} to GCS bucket {} as {}",
            path,
            self.config.bucket,
            uuid
        );

        let file = tokio::fs::File::open(path).await?;
        self.client
            .write_object(&self.bucket_path, &uuid, file)
            .send_unbuffered()
            .await
            .map_err(|e| {
                tracing::error!("Failed to store object from path in GCS: {:?}", e);
                anyhow::anyhow!("GCS write error: {}", e)
            })?;

        tracing::debug!("Stored object {} in bucket {}", uuid, self.config.bucket);
        Ok(StorageIdentity {
            storage_provider: "gcloud".to_string(),
            provider_id: uuid,
            provider_shard: None,
            provider_bucket: Some(self.config.bucket.clone()),
        })
    }
}
