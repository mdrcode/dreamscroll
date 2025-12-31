use anyhow::anyhow;
use argh::FromArgs;

use crate::{
    config, controller, database,
    illumination::{self, Illuminator},
    storage,
};

#[derive(FromArgs)]
#[argh(subcommand, name = "illuminate")]
#[argh(description = "Illuminate a capture without saving to the database.")]
pub struct IlluminateArgs {
    #[argh(positional)]
    #[argh(description = "ID of the capture to illuminate")]
    id: i32,
}

pub async fn run(args: IlluminateArgs) -> anyhow::Result<()> {
    let capture_id = args.id;

    tracing::info!("Starting illumination for capture ID {}", capture_id);

    let (db_config, storage_config) = config::make(config::Env::LocalDev);
    let db = database::connect(db_config).await?;
    let _storage = storage::make(storage_config);

    let capture_info = controller::CaptureInfo::fetch_by_id(&db, capture_id)
        .await
        .map_err(|_| anyhow!("Capture with ID {} not found in database.", capture_id))?;
    tracing::info!("Fetched capture {} from db.", capture_info.id);

    let illuminator = illumination::GrokIlluminator::default();
    let result = illuminator.illuminate(capture_info).await;

    println!("Illumination result for capture ID {}: ", capture_id);
    println!("{}", result);

    Ok(())
}
