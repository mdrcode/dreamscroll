use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use dreamscroll::{database, facility, illumination};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = facility::make_config(facility::Env::LocalDev);
    facility::init_tracing(&config);

    let db = database::connect(config.db_config).await?;
    let db = Arc::new(db);

    let cancel_token = CancellationToken::new();

    let thread_illuminator = {
        let gemini = illumination::make("geministructured");
        let worker = illumination::make_worker(db.clone(), gemini);
        let cancel = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = worker.run() => {}
                _ = cancel.cancelled() => {}
            }
        })
    };

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_illuminator);

    Ok(())
}
