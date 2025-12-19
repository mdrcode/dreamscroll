use std::path::PathBuf;

use argh::FromArgs;
use chrono::Utc;
use sea_orm::ActiveModelTrait;
use sea_orm::Set;

use dreamspot::{db, facility::*, model};

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

    let image_files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap()
                .to_string()
        })
        .collect();

    // connect to the local dev database
    let facility = make_facility(Environment::LocalDev);
    let db_handle = db::connect(facility.db_config()).await?;
    db::run_migrations(&db_handle).await?;

    let mut count = 0;

    for file_name in image_files {
        let capture = model::capture::ActiveModel {
            created_at: Set(Utc::now()),
            ..Default::default()
        };
        let capture = capture.insert(&db_handle.conn).await?;

        let media = model::media::ActiveModel {
            filename: Set(file_name.clone()),
            capture_id: Set(Some(capture.id)),
            ..Default::default()
        };
        let media = media.insert(&db_handle.conn).await?;

        println!(
            "Added capture({}) media({}) added with image: {}",
            capture.id, media.id, file_name
        );
        count += 1;
    }

    println!("Successfully added {} images to the database", count);
    Ok(())
}
