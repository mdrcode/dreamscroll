use argh::FromArgs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::search::{
    Embedder, QueryParams, Searcher,
    gcloud::{GeminiEmbedder, VertexAiSearcher},
};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search_query")]
#[argh(description = "Query Vertex AI Vector Search for the current user")]
pub struct SearchQueryArgs {
    #[argh(positional)]
    #[argh(description = "query text")]
    query: String,

    #[argh(option, default = "20")]
    #[argh(description = "maximum number of results to return")]
    limit: u32,

    #[argh(option)]
    #[argh(description = "optional page token for pagination")]
    page_token: Option<String>,
}

pub async fn run(state: CmdState, args: SearchQueryArgs) -> anyhow::Result<()> {
    let embedder = GeminiEmbedder::from_config(&state.config, state.stg.clone())?;
    let searcher = VertexAiSearcher::from_config(&state.config).await?;

    let query_embedding = embedder.embed_query(&args.query).await?;
    let vector_file_path = write_query_vector_to_temp_json(&query_embedding)?;
    tracing::info!(
        "Generated query embedding with dims={}",
        query_embedding.embedding.len()
    );
    println!("Wrote gcloud vector file: {}", vector_file_path.display());

    let params = QueryParams {
        user_id: 1, // hack TODO fix this up
        limit: args.limit,
        page_token: args.page_token,
    };

    let page = searcher
        .search_query_embedding(&query_embedding, &params)
        .await?;

    println!(
        "Found {} hit(s) for user_id: {} query: {}",
        page.hits.len(),
        params.user_id,
        args.query
    );
    for hit in page.hits {
        println!(
            "id={} capture_id={} score={}",
            hit.doc_id, hit.capture_id, hit.score,
        );
    }

    if let Some(next_page_token) = page.next_page_token {
        println!("next_page_token={}", next_page_token);
    }

    Ok(())
}

fn write_query_vector_to_temp_json(
    query_embedding: &crate::search::QueryEmbedding,
) -> anyhow::Result<std::path::PathBuf> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = std::env::temp_dir().join(format!("query_vector-{}.json", ts));
    let payload = serde_json::json!({
        "dense": {
            "values": query_embedding.embedding,
        }
    });
    let bytes = serde_json::to_vec_pretty(&payload)?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}
