use std::path::PathBuf;

use argh::FromArgs;
use reqwest::StatusCode;

use crate::rest;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "import_digest")]
#[argh(description = "Import captures from an exported digest directory")]
pub struct ImportDigestArgs {
    #[argh(positional)]
    #[argh(description = "path to the export directory containing digest.json")]
    export_dir: PathBuf,

    #[argh(
        option,
        long = "host",
        default = "String::from(\"localhost\")",
        description = "REST API host (default: localhost)"
    )]
    host: String,
}

pub async fn run(_state: CmdState, args: ImportDigestArgs) -> anyhow::Result<()> {
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
    let digest: export_digest::FullDigest = serde_json::from_str(&digest_content)?;

    println!(
        "Found digest v{} exported at {} with {} captures.",
        digest.version,
        digest.exported_at,
        digest.captures.len()
    );

    let (username, password) = auth_helper::prompt_credentials_stdin()?;
    let rest_client = rest::client::Client::connect(&args.host, &username, &password).await?;

    let mut imported_captures = 0;
    let mut skipped_captures = 0;

    for entry in &digest.captures {
        if entry.media_files.is_empty() {
            anyhow::bail!("Capture has no media files");
        }

        // For now, we only import the first media file as the primary capture
        // TODO: Support multiple media files per capture if needed?
        let first_media = &entry.media_files[0];
        let media_path = export_dir.join(first_media);

        if !media_path.exists() {
            anyhow::bail!("Media file not found: {}", media_path.display());
        }

        let media_bytes = tokio::fs::read(&media_path).await?.into();

        match rest_client
            .import_capture(media_bytes, entry.created_at)
            .await
        {
            Err(e) => {
                if e.to_string()
                    .contains(&format!("status {}", StatusCode::CONFLICT.as_u16()))
                {
                    eprintln!(
                        "Skipped capture {} because media hash already exists.",
                        entry.original_id
                    );
                    skipped_captures += 1;
                    continue;
                } else {
                    anyhow::bail!("Failed to import capture {}: {:?}", entry.original_id, e);
                }
            }
            _ => (),
        }

        imported_captures += 1;
    }

    println!(
        "Complete. [digest_contained: {}] [imported: {}] [skipped: {}]",
        digest.captures.len(),
        imported_captures,
        skipped_captures
    );

    Ok(())
}
