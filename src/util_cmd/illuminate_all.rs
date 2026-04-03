use argh::FromArgs;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;

use crate::{illumination, webhook};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "illuminate_all")]
#[argh(
    description = "Illuminate all captures currently missing illumination. Might take some time."
)]
pub struct IlluminateAllArgs {}

pub async fn run(state: CmdState, _args: IlluminateAllArgs) -> anyhow::Result<()> {
    const MAX_CONCURRENT: usize = 2;

    let illuminator = illumination::make_illuminator(&state.config, state.stg.clone());

    let capture_ids = state.service_api.get_captures_need_illum().await?;

    if capture_ids.is_empty() {
        tracing::info!("No captures need illumination, exiting.");
        return Ok(());
    }

    tracing::info!("Found {} captures needing illumination.", capture_ids.len());

    let (job_tx, job_rx) = mpsc::channel::<i32>(2 * MAX_CONCURRENT);

    let job_rx = Arc::new(Mutex::new(job_rx));
    let mut workers: JoinSet<(usize, usize)> = JoinSet::new();

    for _ in 0..MAX_CONCURRENT {
        let service_api = state.service_api.clone();
        let illuminator = illuminator.clone();
        let job_rx = Arc::clone(&job_rx);

        workers.spawn(async move {
            let mut worker_success = 0;
            let mut worker_failed = 0;
            loop {
                let next_capture_id = {
                    let mut rx = job_rx.lock().await;
                    rx.recv().await
                };

                match next_capture_id {
                    Some(id) => {
                        match webhook::logic::illuminate::exec(
                            &service_api,
                            &illuminator,
                            webhook::schema::IlluminationTask { capture_id: id },
                        )
                        .await
                        {
                            Ok(_) => {
                                worker_success += 1;
                            }
                            Err(_) => {
                                worker_failed += 1;
                            }
                        }
                    }
                    None => {
                        break;
                    }
                }
            }

            (worker_success, worker_failed)
        });
    }

    for capture_id in capture_ids {
        job_tx.send(capture_id).await?;
    }

    drop(job_tx);

    let mut num_success = 0;
    let mut num_failed = 0;

    while let Some(result) = workers.join_next().await {
        match result {
            Ok((worker_success, worker_failed)) => {
                num_success += worker_success;
                num_failed += worker_failed;
            }
            Err(err) => {
                tracing::error!(error = ?err, "Illumination worker join error");
            }
        }
    }

    tracing::info!(
        "Illumination complete. Success: {}, Failed: {}.",
        num_success,
        num_failed
    );

    Ok(())
}
