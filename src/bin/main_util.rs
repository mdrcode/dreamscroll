use std::path::PathBuf;

use argh::FromArgs;
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

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "tif"];

#[tokio::main]
async fn main() {
    let args: Args = argh::from_env();

    match args.command {
        Command::Populate(populate_args) => {
            if let Err(e) = run_populate(populate_args).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn run_populate(args: PopulateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let dir = &args.directory;

    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()).into());
    }

    // Connect to the local dev database
    let facility = make_facility(Environment::LocalDev);
    let db_handle = db::connect(facility.db_config()).await?;
    db::run_migrations(&db_handle).await?;

    let mut count = 0;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        let is_image = extension
            .as_ref()
            .map(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
            .unwrap_or(false);

        if !is_image {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid filename")?
            .to_string();

        let media = model::media::ActiveModel {
            filename: Set(filename.clone()),
            capture_id: Set(None),
            ..Default::default()
        };

        media.insert(&db_handle.conn).await?;
        println!("Added: {}", filename);
        count += 1;
    }

    println!("Successfully added {} images to the database", count);
    Ok(())
}
