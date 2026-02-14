use crate::storage::StorageHandle;

use crate::facility;

#[derive(Clone)]
pub struct UrlMaker {
    // "/media" if full URL is something like http://localhost:8000/media/foo.jpg
    pub local_url_prefix: String,
    // e.g. "http://localhost:4443 if running fake-gcs via Docker
    pub gcloud_emulator_endpoint: Option<String>,
}

impl UrlMaker {
    pub fn new(config: &facility::Config) -> Self {
        Self {
            local_url_prefix: config.storage_local_url_prefix.clone(),
            gcloud_emulator_endpoint: config.storage_gcloud_emulator_endpoint.clone(),
        }
    }
}

impl UrlMaker {
    pub fn make_url(&self, id: &StorageHandle) -> String {
        // TODO should do this more performantly
        match id.provider.as_str() {
            "local" => self.make_local_url(id),
            "gcloud" => self.make_gcloud_url(id),
            other => panic!("Unknown storage provider: {}", other),
        }
    }

    pub fn make_local_url(&self, id: &StorageHandle) -> String {
        format!("{}/{}", self.local_url_prefix, id.uuid)
    }

    pub fn make_gcloud_url(&self, id: &StorageHandle) -> String {
        // For emulator, return emulator URL
        if let Some(ref endpoint) = self.gcloud_emulator_endpoint {
            format!(
                "{}/storage/v1/b/{}/o/{}?alt=media",
                endpoint,
                id.bucket.as_ref().unwrap(),
                id.user_shard
                    .as_ref()
                    .map(|shard| format!("{}/{}", shard, id.uuid))
                    .unwrap_or_else(|| id.uuid.clone())
            )
        } else {
            // For production GCS, return the public URL format
            // Note: The object must be publicly accessible for this URL to work
            // TODO: Consider signed URLs for controlled access
            format!(
                "https://storage.googleapis.com/{}/{}",
                id.bucket.as_ref().unwrap(),
                id.user_shard
                    .as_ref()
                    .map(|shard| format!("{}/{}", shard, id.uuid))
                    .unwrap_or_else(|| id.uuid.clone())
            )
        }
    }
}
