use anyhow::anyhow;
use argh::FromArgs;

use crate::search::{
    Embedder, VectorStore,
    gcloud::{GeminiEmbedder, VertexAiVectorStore},
};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search_embed")]
#[argh(
    description = "Embed one or more captures with Gemini and upsert into Vertex AI Vector Search"
)]
pub struct SearchEmbedArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) of the capture(s) to embed and index")]
    ids: Vec<i32>,

    #[argh(switch)]
    #[argh(description = "generate embeddings but do not upsert into Vertex AI Vector Search")]
    no_upsert: bool,
}

pub async fn run(state: CmdState, args: SearchEmbedArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let user = auth_helper::authenticate_user_stdin(&state.db).await?;
    let user_context = user.into();

    let embedder = GeminiEmbedder::from_config(&state.config, state.stg.clone())?;
    let vector_store = if args.no_upsert {
        None
    } else {
        Some(VertexAiVectorStore::from_config(&state.config)?)
    };

    let raw_count = args.ids.len();
    let capture_infos = state
        .user_api
        .get_captures(&user_context, args.ids.clone())
        .await?;
    if capture_infos.is_empty() {
        return Err(anyhow!("No matching captures found for the current user."));
    }

    let mut success_count = 0usize;

    for capture in capture_infos {
        let embed = match embedder.embed_capture(&capture).await {
            Ok(embed) => embed,
            Err(err) => {
                eprintln!("Failed embedding capture {}: {}", capture.id, err);
                continue;
            }
        };

        if let Some(vector_store) = vector_store.as_ref() {
            match vector_store.upsert_capture_embedding(&embed).await {
                Ok(res) => {
                    success_count += 1;
                    println!(
                        "Indexed capture {} -> datapoint {} (dims={})",
                        capture.id, res.id, res.dims
                    );
                }
                Err(err) => {
                    eprintln!("Failed indexing capture {}: {}", capture.id, err);
                }
            }
        } else {
            success_count += 1;
            println!(
                "Embedded capture {} (dims={}) successfully [no upsert]",
                capture.id,
                embed.embedding.len()
            );
        }
    }

    if args.no_upsert {
        println!(
            "Done. Embedded {}/{} capture(s); skipped upsert.",
            success_count, raw_count
        );
    } else {
        println!("Done. Indexed {}/{} capture(s).", success_count, raw_count);
    }

    Ok(())
}
