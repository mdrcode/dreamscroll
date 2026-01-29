use std::path::PathBuf;

use argh::FromArgs;

use crate::{api, database, facility, storage};

use super::auth_helper;
use super::export_digest::FullDigest;

#[derive(FromArgs)]
#[argh(subcommand, name = "import_digest")]
#[argh(description = "Import captures from an exported digest directory")]
pub struct ImportDigestArgs {
    #[argh(positional)]
    #[argh(description = "path to the export directory containing digest.json")]
    export_dir: PathBuf,
}

pub async fn run(config: facility::Config, args: ImportDigestArgs) -> anyhow::Result<()> {
    let export_dir = &args.export_dir;

    if !export_dir.is_dir() {
        anyhow::bail!("'{}' is not a directory", export_dir.display());
    }

    let digest_path = export_dir.join("digest.json");
    if !digest_path.exists() {
        anyhow::bail!("digest.json not found in '{}'", export_dir.display());
    }

    // Read and parse the digest
    let digest_content = std::fs::read_to_string(&digest_path)?;
    let digest: FullDigest = serde_json::from_str(&digest_content)?;

    println!(
        "Found digest v{} exported at {} with {} captures.",
        digest.version,
        digest.exported_at,
        digest.captures.len()
    );

    let db = database::connect(config.db_config).await?;
    let storage = storage::make(config.storage_config);

    let user = auth_helper::authenticate_user_stdin(&db).await?;
    let user_context = user.into();

    let mut imported_captures = 0;

    for entry in &digest.captures {
        if entry.media_files.is_empty() {
            anyhow::bail!("Capture has no media files");
        }

        // For now, we only import the first media file as the primary capture
        // TODO: Support multiple media files per capture if needed
        let first_media = &entry.media_files[0];
        let media_path = export_dir.join(first_media);

        if !media_path.exists() {
            anyhow::bail!("Media file not found: {}", media_path.display());
        }

        let storage_id = storage.store_from_local_path(&media_path)?;

        api::import::import_capture(&db, &user_context, storage_id, entry.created_at)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to import capture (original id: {}): {:?}",
                    entry.original_id,
                    e
                )
            })?;

        imported_captures += 1;
    }

    println!("Successfully imported {} captures.", imported_captures);

    Ok(())
}
