
use crate::storage::StorageIdentity;

#[derive(Clone)]
pub struct StorageUrlMaker {
    web_path_prefix: Option<String>,
    gcloud_emulator_endpoint: Option<String>,
}

impl StorageUrlMaker {
    pub fn new_local(web_path_prefix: String) -> Self {
        Self {
            web_path_prefix: Some(web_path_prefix),
            gcloud_emulator_endpoint: None,
        }
    }
    pub fn new_gcloud(emulator_endpoint: Option<String>) -> Self {
        Self {
            web_path_prefix: None,
            gcloud_emulator_endpoint: emulator_endpoint,
        }
    }
}

impl StorageUrlMaker {
    pub fn make_url(&self, id: &StorageIdentity) -> String {
        match id.storage_provider.as_str() {
            "local" => self.make_local_url(id),
            "gcloud" => self.make_gcloud_url(id),
            other => panic!("Unknown storage provider: {}", other),
        }
    }

    pub fn make_local_url(&self, id: &StorageIdentity) -> String {
        format!(
            "http://localhost:8000/{}/{}",
            self.web_path_prefix.as_ref().unwrap(),
            id.provider_id
        )
    }

    pub fn make_gcloud_url(&self, id: &StorageIdentity) -> String {
        // For emulator, return emulator URL
        if let Some(ref endpoint) = self.gcloud_emulator_endpoint {
            format!(
                "{}/storage/v1/b/{}/o/{}?alt=media",
                endpoint,
                id.provider_bucket.as_ref().unwrap(),
                id.provider_shard
                    .as_ref()
                    .map(|shard| format!("{}/{}", shard, id.provider_id))
                    .unwrap_or_else(|| id.provider_id.clone())
            )
        } else {
            // For production GCS, return the public URL format
            // Note: The object must be publicly accessible for this URL to work
            format!(
                "https://storage.googleapis.com/{}/{}",
                id.provider_bucket.as_ref().unwrap(),
                id.provider_shard
                    .as_ref()
                    .map(|shard| format!("{}/{}", shard, id.provider_id))
                    .unwrap_or_else(|| id.provider_id.clone())
            )
        }
    }
}
