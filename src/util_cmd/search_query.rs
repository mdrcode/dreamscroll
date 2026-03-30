use argh::FromArgs;

use crate::search::{
    Embedder, SearchQueryEmbedding, Searcher,
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
    tracing::info!(
        "Generated query embedding with dims={}",
        query_embedding.embedding.len()
    );

    let query = SearchQueryEmbedding {
        user_id: 1, // hack TODO fix this up
        query_embedding: query_embedding.embedding,
        limit: args.limit,
        page_token: args.page_token,
    };

    let page = searcher.search_query_embedding(&query).await?;

    println!("Found {} hit(s) for query: {}", page.hits.len(), args.query);
    for hit in page.hits {
        println!(
            "id={} capture_id={} score={}",
            hit.corpus_doc_id, hit.capture_id, hit.score,
        );
    }

    if let Some(next_page_token) = page.next_page_token {
        println!("next_page_token={}", next_page_token);
    }

    Ok(())
}
