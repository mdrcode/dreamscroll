use std::path::Path;
use std::path::PathBuf;

use argh::FromArgs;

use crate::{common, model};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "export_uniq")]
#[argh(description = "Export images from db to a directory, avoiding content duplicates")]
pub struct ExportUniqArgs {
    #[argh(positional)]
    #[argh(description = "directory path for exported images (can contain existing images)")]
    directory: PathBuf,
}

pub async fn run(state: CmdState, args: ExportUniqArgs) -> anyhow::Result<()> {
    let export_dir = &args.directory;

    if !export_dir.is_dir() {
        std::fs::create_dir_all(export_dir)?;
    }

    // First, compute hashes for existing content of the export dir
    let mut existing_hashes = std::collections::HashSet::new();
    for entry in std::fs::read_dir(export_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let hash = common::compute_file_hash(&path)?;
            existing_hashes.insert(hash);
        }
    }
    println!(
        "Found {} unique binary hashes already in export directory.",
        existing_hashes.len()
    );

    let medias = model::media::Entity::load()
        .all(&state.db.conn)
        .await
        .expect("Failed to fetch medias from db.")
        .into_iter()
        .collect::<Vec<_>>();

    println!("Retrieved {} media for potential export.", medias.len());

    let mut skipped = 0;
    let mut exported = 0;

    for media in medias {
        let storage_handle = media.storage_uuid.to_string();

        // TODO obviously this will break when not using local storage...
        let storage_path = PathBuf::from(format!("localdev/media/{}", &storage_handle));
        let hash = common::compute_file_hash(&storage_path)?;

        if existing_hashes.contains(&hash) {
            skipped += 1;
            continue;
        }

        let filename = Path::new(&*storage_handle);
        let dest_path = export_dir.join(filename);

        std::fs::copy(&storage_path, &dest_path)?;
        existing_hashes.insert(hash);
        exported += 1;
    }

    println!(
        "Exported {} new images, skipped {} duplicates.",
        exported, skipped
    );

    Ok(())
}
