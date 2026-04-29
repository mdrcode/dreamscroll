use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;
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
    pub async fn new(
        emulator_endpoint: Option<String>,
        prod_endpoint: Option<String>,
        bucket_name: String,
    ) -> Self {
        let mut builder = Storage::builder();

        // Infer that we are using the emulator if emulator_endpoint is set.
        // This takes precedence over prod_endpoint if both are set.
        if let Some(emulator) = emulator_endpoint {
            builder = builder
                .with_endpoint(emulator.clone())
                .with_credentials(credentials::anonymous::Builder::default().build());
            tracing::info!("GCloudStorageProvider using emulator at: {}", emulator);
        } else if let Some(prod) = prod_endpoint {
            builder = builder.with_endpoint(prod.clone());
            tracing::info!("GCloudStorageProvider using production at: {}", prod);
        } else {
            tracing::info!("GCloudStorageProvider using production with default endpoint");
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

pub fn make_object_key(uuid: Uuid, shard: &str, ext: Option<&str>) -> String {
    match ext {
        Some(e) => format!("{}/{}.{}", shard, uuid, e),
        None => format!("{}/{}", shard, uuid),
    }
}

#[async_trait]
impl provider::StorageProvider for GCloudStorageProvider {
    async fn store_bytes(
        &self,
        bytes: Bytes,
        user_shard: &str,
        ext: Option<&str>,
    ) -> anyhow::Result<StorageHandle> {
        let uuid = Uuid::new_v4();
        let object_key = make_object_key(uuid, user_shard, ext);
        let n_bytes = bytes.len();

        // BUG currently if the GCS client cannot connect to the emulator
        // endpoint, it will hang indefinitely rather than timeout :-/
        let write_start = std::time::Instant::now();
        self.gcloud_client
            .write_object(&self.bucket_path, &object_key, bytes)
            //.with_resumable_upload_threshold(5 * 1024 * 1024_usize) // TODO investigate this?
            .send_unbuffered()
            .await
            .map_err(|e| {
                tracing::error!("Failed to store object in GCS: {:?}", e);
                anyhow::anyhow!("Failed to store object in GCS: {}", e)
            })?;
        let write_duration = write_start.elapsed();

        tracing::info!(
            bucket = self.bucket_name,
            object_key,
            bytes = n_bytes,
            duration_ms = write_duration.as_millis(),
            "Stored object {} in bucket {} in {} ms",
            object_key,
            self.bucket_name,
            write_duration.as_millis()
        );

        Ok(StorageHandle {
            provider: "gcloud".to_string(),
            bucket: Some(self.bucket_name.clone()),
            user_shard: user_shard.to_string(),
            uuid,
            extension: ext.map(|s| s.to_string()),
        })
    }

    async fn store_from_local_path(
        &self,
        path: &Path,
        user_shard: &str,
        ext: Option<&str>,
    ) -> anyhow::Result<StorageHandle> {
        let uuid = Uuid::new_v4();
        let object_key = make_object_key(uuid, user_shard, ext);

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
                anyhow::anyhow!("Failed to store object from path in GCS: {}", e)
            })?;

        tracing::debug!(
            "Stored object {} in bucket {}",
            object_key,
            self.bucket_name
        );
        Ok(StorageHandle {
            provider: "gcloud".to_string(),
            uuid,
            user_shard: user_shard.to_string(),
            bucket: Some(self.bucket_name.clone()),
            extension: ext.map(|s| s.to_string()),
        })
    }

    async fn retrieve_bytes(&self, h: &StorageHandle) -> anyhow::Result<Vec<u8>> {
        let object_key = make_object_key(h.uuid, &h.user_shard, h.extension.as_deref());

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

    fn make_prod_uri(&self, handle: &StorageHandle) -> anyhow::Result<String> {
        let key = make_object_key(handle.uuid, &handle.user_shard, handle.extension.as_deref());

        let bucket = handle
            .bucket
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Cannot construct GCS URI without a bucket"))?;

        Ok(format!("gs://{}/{}", bucket, key))
    }
}
