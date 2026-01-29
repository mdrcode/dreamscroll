use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use dreamscroll::{auth, database, facility, illumination};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = facility::make_config(facility::Env::LocalDev);
    facility::init_tracing(&config);

    let db = database::connect(config.db_config).await?;
    let db = Arc::new(db);

    let jwt_config = Arc::new(config.jwt_config);

    let cancel_token = CancellationToken::new();

    let ill_context = auth::Context::from_service_credentials(
        &jwt_config,
        // For local dev, there are no true secrets, so just create a token on the fly
        jwt_config.create_service_token("illuminator_worker")?,
    )?;
    let thread_illuminator = {
        let gemini = illumination::make_illuminator("geministructured");
        let worker = illumination::make_worker(db.clone(), ill_context, gemini);
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
