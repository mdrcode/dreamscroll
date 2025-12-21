use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

use argh::FromArgs;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

use dreamspot::{
    config, database,
    model::{self, media},
    storage,
};

#[derive(FromArgs)]
#[argh(description = "dreamscroll admin utility")]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Import(ImportArgs),
    ExportUniq(ExportUniqArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "import")]
#[argh(description = "Import assets from a directory into the db")]
struct ImportArgs {
    #[argh(positional)]
    #[argh(description = "directory path containing images to add")]
    directory: PathBuf,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "export_uniq")]
#[argh(description = "Export images from db to a directory, avoiding content duplicates")]
struct ExportUniqArgs {
    #[argh(positional)]
    #[argh(description = "directory path for exported images (can contain existing images)")]
    directory: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    match args.command {
        Command::Import(import_args) => run_import(import_args).await?,
        Command::ExportUniq(export_uniq_args) => run_export_uniq(export_uniq_args).await?,
    }

    Ok(())
}

async fn run_import(args: ImportArgs) -> anyhow::Result<()> {
    let dir = &args.directory;

    if !dir.is_dir() {
        anyhow::bail!("'{}' is not a directory", dir.display());
    }

    let file_paths = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .map(|path| dir.join(&path));

    let (db_config, storage_config) = config::make_local_dev();

    // connect to the local dev database and storage provider
    let db = database::connect(db_config).await?;
    database::run_migrations(&db).await?;

    let storage = storage::make(storage_config);

    let mut imported = 0;

    for img_path in file_paths {
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
        media.insert(&db.conn).await?;

        imported += 1;
    }

    println!("Added {} images to the database.", imported);
    Ok(())
}

fn compute_file_hash(path: &Path) -> anyhow::Result<blake3::Hash> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut reader)?;
    let hash = hasher.finalize();
    Ok(hash)
}

async fn run_export_uniq(args: ExportUniqArgs) -> anyhow::Result<()> {
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
            let hash = compute_file_hash(&path)?;
            existing_hashes.insert(hash);
        }
    }
    println!(
        "Found {} unique binary hashes already in export directory.",
        existing_hashes.len()
    );

    let (db_config, _) = config::make_local_dev();
    let db = database::connect(db_config).await?;
    database::run_migrations(&db).await?;

    let medias = media::Entity::find()
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
        let hash = compute_file_hash(&storage_path)?;

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
