use anyhow::anyhow;
use argh::FromArgs;

use crate::search::{
    Embedder, VectorStore,
    gcloud::{GeminiEmbedder, VertexAiVectorStore},
};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search_embed")]
#[argh(description = "Embed one or more captures and upsert into Vertex AI Vector Search")]
pub struct SearchEmbedArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) of the capture(s) to embed and index")]
    ids: Vec<i32>,
}

pub async fn run(state: CmdState, args: SearchEmbedArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let user = auth_helper::authenticate_user_stdin(&state.db).await?;
    let user_context = user.into();

    let embedder = GeminiEmbedder::from_config(&state.config, state.stg.clone())?;
    let vector_store = VertexAiVectorStore::from_config(&state.config)?;

    let raw_count = args.ids.len();
    let captures = state
        .user_api
        .get_captures(&user_context, args.ids.clone())
        .await?;
    if captures.is_empty() {
        return Err(anyhow!("No matching captures found for the current user."));
    }

    let mut success_count = 0usize;

    for capture in captures {
        let embedding = match embedder.embed_capture(&capture).await {
            Ok(embedding) => embedding,
            Err(err) => {
                eprintln!("Failed embedding capture {}: {}", capture.id, err);
                continue;
            }
        };

        match vector_store.upsert_capture_embedding(&embedding).await {
            Ok(res) => {
                success_count += 1;
                println!(
                    "Indexed capture {} -> datapoint {} (dims={})",
                    capture.id, res.datapoint_id, res.embedding_dimensions
                );
            }
            Err(err) => {
                eprintln!("Failed indexing capture {}: {}", capture.id, err);
            }
        }
    }

    println!(
        "Done. Indexed {}/{} capture(s).",
        success_count, raw_count
    );

    Ok(())
}
