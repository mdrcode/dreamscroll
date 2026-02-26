use crate::storage::StorageHandle;

use crate::facility;

#[derive(Clone)]
pub struct UrlMaker {
    // "/media" if full URL is something like http://localhost:8000/media/foo.jpg
    local_url_prefix: Option<String>,
    // e.g. "http://localhost:4443 if running fake-gcs via Docker
    gcloud_emulator_endpoint: Option<String>,
}

impl UrlMaker {
    pub fn from_config(config: &facility::Config) -> Self {
        Self {
            local_url_prefix: config.storage_local_url_prefix.clone(),
            gcloud_emulator_endpoint: config.storage_gcloud_emulator.clone(),
        }
    }
}

impl UrlMaker {
    pub fn make_url(&self, id: &StorageHandle) -> String {
        // TODO should do this more performantly
        match id.provider.as_str() {
            "local" => self.make_local_url(id),
            "gcloud" => self.make_gcloud_url(id),
            other => unimplemented!("Unknown storage provider: {}", other),
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

    pub fn make_gcloud_url(&self, id: &StorageHandle) -> String {
        // For emulator, return emulator URL
        if let Some(_) = self.gcloud_emulator_endpoint {
            format!(
                "{}/storage/v1/b/{}/o/{}%2F{}{}?alt=media",
                "http://localhost:4443", // TODO hardcode localhost for now
                //endpoint,
                id.bucket.as_ref().unwrap(),
                id.user_shard,
                id.uuid,
                id.extension
                    .as_ref()
                    .map(|ext| format!(".{}", ext))
                    .unwrap_or_default()
            )
        } else {
            // For production GCS, return the public URL format
            // Note: The object must be publicly accessible for this URL to work
            // TODO: Consider signed URLs or signed cookies for controlled access
            format!(
                "https://storage.googleapis.com/{}/{}/{}{}",
                id.bucket.as_ref().unwrap(),
                id.user_shard,
                id.uuid,
                id.extension
                    .as_ref()
                    .map(|ext| format!(".{}", ext))
                    .unwrap_or_default()
            )
        }
    }
}
