use crate::storage::StorageIdentity;

#[derive(Clone, Default)]
pub struct StorageUrlMaker {
    local_url_prefix: Option<String>,
    gcloud_emulator_endpoint: Option<String>,
}

impl StorageUrlMaker {
    pub fn with_local_url_prefix(mut self, prefix: String) -> Self {
        self.local_url_prefix = Some(prefix);
        self
    }

    pub fn with_gcloud_emulator_endpoint(mut self, endpoint: String) -> Self {
        self.gcloud_emulator_endpoint = Some(endpoint);
        self
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
            "{}/{}",
            self.local_url_prefix.as_ref().unwrap(),
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
            // TODO: Consider signed URLs for controlled access
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
