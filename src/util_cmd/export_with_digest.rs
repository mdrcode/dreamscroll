use std::path::PathBuf;

use argh::FromArgs;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::{database, facility, model};

#[derive(FromArgs)]
#[argh(subcommand, name = "export_with_digest")]
#[argh(description = "Export all captures with images and a JSON digest for later import")]
pub struct ExportWithDigestArgs {
    #[argh(positional)]
    #[argh(description = "root directory where export folder will be created")]
    root_dir: PathBuf,
}

/// Represents a single capture in the export digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureDigestEntry {
    /// Original capture ID (for reference, not used on import)
    pub original_id: i32,
    /// User ID who created the capture
    pub user_id: i32,
    /// When the capture was created (preserved across export/import)
    pub created_at: DateTime<Utc>,
    /// Media files associated with this capture (filenames in export folder)
    pub media_files: Vec<String>,
}

/// The complete export digest containing all captures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDigest {
    /// Version of the digest format
    pub version: u32,
    /// When the export was created
    pub exported_at: DateTime<Utc>,
    /// All captures in the export
    pub captures: Vec<CaptureDigestEntry>,
}

impl ExportDigest {
    pub fn new() -> Self {
        Self {
            version: 1,
            exported_at: Utc::now(),
            captures: Vec::new(),
        }
    }
}

pub async fn run(config: facility::Config, args: ExportWithDigestArgs) -> anyhow::Result<()> {
    let root_dir = &args.root_dir;

    // Create export folder with timestamp
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let export_dir = root_dir.join(format!("dreamscroll_export_{}", timestamp));

    if !root_dir.is_dir() {
        std::fs::create_dir_all(root_dir)?;
    }

    std::fs::create_dir_all(&export_dir)?;
    println!("Created export directory: {}", export_dir.display());

    let db = database::connect(config.db_config).await?;

    // Fetch all captures with their related data
    let captures = model::capture::Entity::load().all(&db.conn).await?;

    println!("Found {} captures to export.", captures.len());

    let mut digest = ExportDigest::new();
    let mut total_media = 0;

    for capture in captures {
        // Fetch media for this capture
        let medias = model::media::Entity::find()
            .filter(model::media::Column::CaptureId.eq(Some(capture.id)))
            .all(&db.conn)
            .await?;

        let mut media_files = Vec::new();

        for media in &medias {
            let storage_id = &media.filename;

            // TODO: This assumes local storage - should use storage abstraction
            let storage_path = PathBuf::from(format!("localdev/media/{}", storage_id));

            if storage_path.exists() {
                // Copy to export directory
                let dest_path = export_dir.join(storage_id);
                std::fs::copy(&storage_path, &dest_path)?;
                media_files.push(storage_id.clone());
                total_media += 1;
            } else {
                eprintln!(
                    "Warning: Media file not found for capture {}: {}",
                    capture.id,
                    storage_path.display()
                );
            }
        }

        digest.captures.push(CaptureDigestEntry {
            original_id: capture.id,
            user_id: capture.user_id,
            created_at: capture.created_at,
            media_files,
        });
    }

    // Write digest.json
    let digest_path = export_dir.join("digest.json");
    let digest_json = serde_json::to_string_pretty(&digest)?;
    std::fs::write(&digest_path, &digest_json)?;

    println!(
        "Export complete: {} captures, {} media files.",
        digest.captures.len(),
        total_media
    );
    println!("Digest written to: {}", digest_path.display());

    Ok(())
}
