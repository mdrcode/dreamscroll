use anyhow::anyhow;
use argh::FromArgs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::search::{self, prelude::*};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search_index")]
#[argh(
    description = "Embed one or more captures with Gemini and upsert into Vertex AI Vector Search"
)]
pub struct SearchIndexArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) of the capture(s) to embed and index (omit when --all is used)")]
    ids: Vec<i32>,

    #[argh(switch)]
    #[argh(description = "index all captures for the authenticated user")]
    all: bool,

    #[argh(switch)]
    #[argh(description = "generate embeddings but do not upsert into Vertex AI Vector Search")]
    no_upsert: bool,
}

pub async fn run(state: CmdState, args: SearchIndexArgs) -> anyhow::Result<()> {
    if args.all && !args.ids.is_empty() {
        return Err(anyhow!(
            "Provide either --all or explicit capture IDs, not both."
        ));
    }
    if !args.all && args.ids.is_empty() {
        return Err(anyhow!(
            "At least one capture ID must be provided unless using --all."
        ));
    }

    let user = auth_helper::authenticate_user_stdin(&state.db).await?;
    let user_context = user.into();

    let (raw_count, capture_infos) = if args.all {
        let captures = state
            .user_api
            .get_timeline_captures(&user_context, 1000)
            .await?;
        let count = captures.len();
        tracing::info!(count, "Fetched captures for --all indexing run");
        (count, captures)
    } else {
        let captures = state
            .user_api
            .get_captures(&user_context, args.ids.clone())
            .await?;
        (args.ids.len(), captures)
    };
    if capture_infos.is_empty() {
        return Err(anyhow!("No matching captures found for the current user."));
    }

    let embedder = search::gcloud::GeminiEmbedder::from_config(&state.config)?;
    let vector_store = if args.no_upsert {
        None
    } else {
        Some(search::gcloud::VertexVectorStore::from_config(&state.config).await?)
    };

    let retrieved_count = capture_infos.len();
    let mut success_count = 0usize;
    let mut last_vector: Option<(i32, Vec<f32>)> = None;

    for capture in capture_infos {
        let input = search::make_capture_info_embed_input(state.stg.as_ref(), &capture).await?;
        let embedding = match embedder.embed_object(input).await {
            Ok(embedding) => {
                tracing::debug!(
                    "Embedded capture {} (dims={}) successfully",
                    capture.id,
                    embedding.len()
                );
                embedding
            }
            Err(err) => {
                tracing::error!("Failed embedding capture {}: {}", capture.id, err);
                continue;
            }
        };

        if let Some(vector_store) = vector_store.as_ref() {
            match vector_store
                .upsert_object_embedding(&capture, &embedding)
                .await
            {
                Ok(res) => {
                    tracing::debug!(
                        "Indexed capture {} -> datapoint {} (dims={})",
                        capture.id,
                        res.id,
                        res.dims
                    );
                    last_vector = Some((capture.id, embedding.as_slice().to_vec()));
                }
                Err(err) => {
                    tracing::error!("Failed indexing capture {}: {}", capture.id, err);
                    continue;
                }
            }
        } else {
            last_vector = Some((capture.id, embedding.as_slice().to_vec()));
        }

        success_count += 1;
    }

    if args.no_upsert {
        println!(
            "Done. Embedded {}/{} capture(s); skipped upsert.",
            success_count, raw_count
        );
    } else {
        println!(
            "Done. Asked: {} Retrieved: {} Indexed {} capture(s).",
            raw_count, retrieved_count, success_count
        );
    }

    if let Some((capture_id, vector)) = last_vector {
        let vector_path = write_dense_vector_tmp_json(capture_id, &vector)?;
        println!(
            "Last indexed capture vector file path: {}",
            vector_path.display()
        );
    }

    Ok(())
}

fn write_dense_vector_tmp_json(capture_id: i32, vector: &[f32]) -> anyhow::Result<PathBuf> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = std::env::temp_dir().join(format!("capture-{}-vector-{}.json", capture_id, ts));
    let payload = serde_json::json!({
        "dense": {
            "values": vector,
        }
    });
    let bytes = serde_json::to_vec_pretty(&payload)?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}
