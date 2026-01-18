use std::path::PathBuf;

use argh::FromArgs;

use crate::{api, database, facility, storage};

#[derive(FromArgs)]
#[argh(subcommand, name = "import")]
#[argh(description = "Import assets from a directory into the db")]
pub struct ImportArgs {
    #[argh(positional)]
    #[argh(description = "path to directory containing images to add")]
    directory: PathBuf,
}

pub async fn run(config: facility::Config, args: ImportArgs) -> anyhow::Result<()> {
    let dir = &args.directory;

    tracing::info!("Starting import from directory {}", dir.display());

    if !dir.is_dir() {
        anyhow::bail!("'{}' is not a directory", dir.display());
    }

    let paths = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();

    tracing::info!("Found {} to import from {}.", paths.len(), dir.display());

    let db = database::connect(config.db_config).await?;
    let storage = storage::make(config.storage_config);
    let mut imported = 0;

    for path in paths {
        let storage_id = storage.store_from_local_path(&path)?;

        let capture_info = api::insert_capture(&db, storage_id.clone()).await?;

        tracing::info!(
            "Imported new capture {} with storage id {} from path {}",
            capture_info.id,
            storage_id,
            path.display(),
        );

        imported += 1;
    }

    println!("Added {} images to the database.", imported);
    Ok(())
}
