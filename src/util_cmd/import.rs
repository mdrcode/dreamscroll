use std::path::PathBuf;

use argh::FromArgs;

use crate::auth;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "import")]
#[argh(description = "Import assets from a directory into the db")]
pub struct ImportArgs {
    #[argh(option)]
    #[argh(description = "path to directory containing images to add")]
    directory: PathBuf,
}

pub async fn run(state: CmdState, args: ImportArgs) -> anyhow::Result<()> {
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

    let user = auth_helper::authenticate_user_stdin(&state.db).await?;
    let user_shard = user.storage_shard().to_owned();
    let user_context = auth::Context::from(user);

    let mut imported = 0;

    for path in paths {
        let storage_id = state.stg.store_from_local_path(&path, &user_shard).await?;

        let capture_info = state
            .user_api
            .insert_capture(&user_context, storage_id.clone())
            .await?;

        tracing::info!(
            "Imported new capture {} media with storage {}",
            capture_info.id,
            storage_id,
        );

        imported += 1;
    }

    println!("Added {} images to the database.", imported);
    Ok(())
}
