use std::path::PathBuf;

use anyhow::Context;
use argh::FromArgs;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "export_digest")]
#[argh(description = "Export all captures with images and a JSON digest for later import")]
pub struct ExportDigestArgs {
    #[argh(positional)]
    #[argh(description = "root directory where export folder will be created")]
    root_dir: PathBuf,
}

/// Represents a single capture in the export digest.
/// Note that this does NOT include user_id, so that captures can be imported from
/// any user in environment A to any user in environment B.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureDigestEntry {
    /// Original capture ID (for reference, not used on import)
    pub original_id: i32,
    /// When the capture was created (preserved across export/import)
    pub created_at: DateTime<Utc>,
    /// Media files associated with this capture (filenames in export folder)
    pub media_files: Vec<String>,
}

/// The complete export digest containing all captures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullDigest {
    /// Version of the digest format
    pub version: u32,
    /// When the export was created
    pub exported_at: DateTime<Utc>,
    /// All captures in the export
    pub captures: Vec<CaptureDigestEntry>,
}

impl FullDigest {
    pub fn new() -> Self {
        Self {
            version: 1,
            exported_at: Utc::now(),
            captures: Vec::new(),
        }
    }
}

pub async fn run(state: CmdState, args: ExportDigestArgs) -> anyhow::Result<()> {
    let media_http = reqwest::Client::new();

    // Create export folder with timestamp
    let root_dir = &args.root_dir;
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let export_dir = root_dir.join(format!("dreamscroll_export_{}", timestamp));
    std::fs::create_dir_all(&export_dir)?;
    println!("Created export directory: {}", export_dir.display());

    // Fetch all capture_infos from REST API for this user
    let capture_infos = state.rest_client.get_captures(None).await?;

    println!("Using REST host: {}", state.rest_host);

    println!("Found {} captures to export.", capture_infos.len());

    let mut digest = FullDigest::new();

    for capture in capture_infos {
        let media = match capture.medias.first() {
            Some(m) => m,
            None => {
                eprintln!("Warning: Capture {} has no media, skipping", capture.id);
                continue;
            }
        };

        let media_response = media_http
            .get(&media.url)
            .send()
            .await
            .with_context(|| format!("failed to fetch media from {}", media.url))?;

        let status = media_response.status();
        if !status.is_success() {
            anyhow::bail!(
                "failed to fetch media for capture {} from {}: status {}",
                capture.id,
                media.url,
                status
            );
        }

        let bytes = media_response
            .bytes()
            .await
            .with_context(|| format!("failed reading media body from {}", media.url))?;

        // Copy to export directory
        let dest_path = export_dir
            .join(media.storage_uuid.to_string())
            .with_extension(&media.storage_extension.as_deref().unwrap_or_default());
        std::fs::write(&dest_path, bytes)?;

        digest.captures.push(CaptureDigestEntry {
            original_id: capture.id,
            created_at: capture.created_at,
            media_files: vec![dest_path.file_name().unwrap().to_string_lossy().to_string()],
        });
    }

    // Write digest.json
    let digest_path = export_dir.join("digest.json");
    let digest_json = serde_json::to_string_pretty(&digest)?;
    std::fs::write(&digest_path, &digest_json)?;

    println!("Export complete: {} captures.", digest.captures.len(),);
    println!("Digest written to: {}", digest_path.display());

    Ok(())
}
