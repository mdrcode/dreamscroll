use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub struct ImageInfo {
    pub filename: String,
    pub timestamp: String,
}

pub fn collect_images() -> Vec<ImageInfo> {
    let mut images = Vec::new();
    if let Ok(mut entries) = std::fs::read_dir("localdev/uploads") {
        while let Some(entry_result) = entries.next() {
            if let Ok(entry) = entry_result {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(mtime) = metadata.modified() {
                        let datetime: DateTime<Utc> = mtime.into();
                        let filename = entry.file_name().to_string_lossy().to_string();
                        let timestamp = datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string();
                        images.push(ImageInfo {
                            filename,
                            timestamp,
                        });
                    }
                }
            }
        }
    }
    images
}
