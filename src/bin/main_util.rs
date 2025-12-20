use std::path::PathBuf;

use argh::FromArgs;
use chrono::Utc;
use sea_orm::ActiveModelTrait;
use sea_orm::Set;

use dreamspot::{config, database, model, storage};

#[derive(FromArgs)]
#[argh(description = "dreamscroll admin utility")]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Populate(PopulateArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "populate")]
#[argh(description = "Populate the database with images from a directory")]
struct PopulateArgs {
    #[argh(positional)]
    #[argh(description = "directory path containing images to add")]
    directory: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    match args.command {
        Command::Populate(populate_args) => run_populate(populate_args).await?,
    }

    Ok(())
}

async fn run_populate(args: PopulateArgs) -> anyhow::Result<()> {
    const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "tif"];

    let dir = &args.directory;

    if !dir.is_dir() {
        anyhow::bail!("'{}' is not a directory", dir.display());
    }

    let image_paths: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|path| dir.join(&path))
        .collect();

    let (db_config, storage_config) = config::make_local_dev();

    // connect to the local dev database and storage provider
    let db = database::connect(db_config).await?;
    database::run_migrations(&db).await?;

    let storage = storage::make(storage_config);

    let mut count = 0;

    for img_path in image_paths {
        let storage_id = storage.store_from_local_path(&img_path)?;

        let capture = model::capture::ActiveModel {
            created_at: Set(Utc::now()),
            ..Default::default()
        };
        let capture = capture.insert(&db.conn).await?;

        let media = model::media::ActiveModel {
            filename: Set(storage_id.clone()),
            capture_id: Set(Some(capture.id)),
            ..Default::default()
        };
        let media = media.insert(&db.conn).await?;

        println!(
            "Added capture({}) media({}) with storage id: {}",
            capture.id, media.id, storage_id
        );
        count += 1;
    }

    println!("Successfully added {} images to the database.", count);
    Ok(())
}
