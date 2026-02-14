use std::path::PathBuf;

use argh::FromArgs;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::storage;

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
    let user = auth_helper::authenticate_user_stdin(&state.db).await?;

    // Create export folder with timestamp
    let root_dir = &args.root_dir;
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let export_dir = root_dir.join(format!("dreamscroll_export_{}", timestamp));
    std::fs::create_dir_all(&export_dir)?;
    println!("Created export directory: {}", export_dir.display());

    // Fetch all capture_infos from API for this user
    let capture_infos = state.user_api.get_captures(&user.into(), None).await?;

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

        let storage_handle = storage::StorageHandle::from(media);
        let bytes = state.stg.retrieve_bytes(&storage_handle).await?;

        // Copy to export directory
        let dest_path = export_dir.join(media.storage_uuid.to_string());
        std::fs::write(&dest_path, bytes)?;

        digest.captures.push(CaptureDigestEntry {
            original_id: capture.id,
            created_at: capture.created_at,
            media_files: vec![media.storage_uuid.to_string()],
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
