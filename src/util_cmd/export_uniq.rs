use std::path::Path;
use std::path::PathBuf;

use argh::FromArgs;

use crate::{common, database, facility, model::media};

#[derive(FromArgs)]
#[argh(subcommand, name = "export_uniq")]
#[argh(description = "Export images from db to a directory, avoiding content duplicates")]
pub struct ExportUniqArgs {
    #[argh(positional)]
    #[argh(description = "directory path for exported images (can contain existing images)")]
    directory: PathBuf,
}

pub async fn run(config: facility::Config, args: ExportUniqArgs) -> anyhow::Result<()> {
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

    let db = database::connect(config.db_config).await?;

    let medias = media::Entity::load()
        .all(&db.conn)
        .await
        .expect("Failed to fetch medias from db.")
        .into_iter()
        .collect::<Vec<_>>();

    println!("Retrieved {} media for potential export.", medias.len());

    let mut skipped = 0;
    let mut exported = 0;

    for media in medias {
        let storage_id = &media.filename;

        // TODO obviously this will break when not using local storage...
        let storage_path = PathBuf::from(format!("localdev/media/{}", &storage_id));
        let hash = common::compute_file_hash(&storage_path)?;

        if existing_hashes.contains(&hash) {
            skipped += 1;
            continue;
        }

        let filename = Path::new(storage_id);
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
