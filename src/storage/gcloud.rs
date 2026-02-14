use std::path::PathBuf;

use async_trait::async_trait;
use axum::body::Bytes;
use google_cloud_auth::credentials;
use google_cloud_storage::client::Storage;
use uuid::Uuid;

use super::*;

#[derive(Clone)]
pub struct GCloudStorageProvider {
    bucket_name: String,
    bucket_path: String, // format for the API: "projects/_/buckets/{bucket_name}"
    gcloud_client: Storage,
}

impl GCloudStorageProvider {
    pub async fn new(emulator_endpoint: Option<String>, bucket_name: String) -> Self {
        let mut builder = Storage::builder();

        // Infer that we are using the emulator if .emulator_endpoint is set
        if let Some(endpoint) = emulator_endpoint {
            builder = builder
                .with_endpoint(endpoint.clone())
                .with_credentials(credentials::anonymous::Builder::default().build());
        }

        let gcloud_client = builder
            .build()
            .await
            .expect("Failed to create GCloud Storage client");

        let bucket_path = format!("projects/_/buckets/{}", bucket_name);

        Self {
            bucket_name,
            bucket_path,
            gcloud_client,
        }
    }
}

#[async_trait]
impl provider::StorageProvider for GCloudStorageProvider {
    async fn store_bytes(&self, data: &[u8], user_shard: &str) -> anyhow::Result<StorageHandle> {
        let uuid = Uuid::new_v4();
        let object_key = format!("{}/{}", user_shard, uuid);
        let bytes_data = Bytes::copy_from_slice(data);

        self.gcloud_client
            .write_object(&self.bucket_path, &object_key, bytes_data)
            .send_buffered()
            .await
            .map_err(|e| {
                tracing::error!("Failed to store object in GCS: {:?}", e);
                anyhow::anyhow!("Failed to store object in GCS: {}", e)
            })?;

        tracing::debug!("Stored object {} in bucket {}", object_key, self.bucket_name);
        Ok(StorageHandle {
            provider: "gcloud".to_string(),
            uuid,
            user_shard: Some(user_shard.to_string()),
            bucket: Some(self.bucket_name.clone()),
        })
    }

    async fn store_from_local_path(&self, path: &PathBuf, user_shard: &str) -> anyhow::Result<StorageHandle> {
        let uuid = Uuid::new_v4();
        let object_key = format!("{}/{}", user_shard, uuid);

        tracing::info!(
            "Storing from local path {:?} to GCS bucket {} as {}",
            path,
            self.bucket_name,
            object_key
        );

        let file = tokio::fs::File::open(path).await?;
        self.gcloud_client
            .write_object(&self.bucket_path, &object_key, file)
            .send_unbuffered()
            .await
            .map_err(|e| {
                tracing::error!("Failed to store object from path in GCS: {:?}", e);
                anyhow::anyhow!("GCS write error: {}", e)
            })?;

        tracing::debug!("Stored object {} in bucket {}", object_key, self.bucket_name);
        Ok(StorageHandle {
            provider: "gcloud".to_string(),
            uuid,
            user_shard: Some(user_shard.to_string()),
            bucket: Some(self.bucket_name.clone()),
        })
    }

    async fn retrieve_bytes(&self, id: &StorageHandle) -> anyhow::Result<Vec<u8>> {
        let object_key = match &id.user_shard {
            Some(shard) => format!("{}/{}", shard, id.uuid),
            None => id.uuid.to_string(),
        };

        let mut reader = self
            .gcloud_client
            .read_object(&self.bucket_path, &object_key)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Failed to read object from GCS: {:?}", e);
                anyhow::anyhow!("Failed to read object from GCS: {}", e)
            })?;

        let mut contents = Vec::new();
        while let Some(chunk) = reader.next().await.transpose()? {
            contents.extend_from_slice(&chunk);
        }
        Ok(contents)
    }
}
