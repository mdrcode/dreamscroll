use argh::FromArgs;
use serde_json::json;

use crate::search::{self, *};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search")]
#[argh(description = "Search captures by text query")]
pub struct SearchArgs {
    #[argh(positional)]
    #[argh(description = "query text")]
    query: String,

    #[argh(option, default = "20")]
    #[argh(description = "maximum number of results to return")]
    limit: u32,

    #[argh(option)]
    #[argh(description = "optional page token for pagination")]
    page_token: Option<String>,

    #[argh(switch)]
    #[argh(description = "run text-only search")]
    text_only: bool,

    #[argh(switch)]
    #[argh(description = "run vector-only search")]
    vector_only: bool,
}

pub async fn run(state: CmdState, args: SearchArgs) -> anyhow::Result<()> {
    let searcher = search::gcloud::VertexVectorSearcher::from_config(&state.config).await?;

    if args.text_only && args.vector_only {
        anyhow::bail!("Choose at most one mode: --text-only or --vector-only");
    }

    let params = QueryParams {
        base_filter: json!({
            "user_id": {
                "$eq": "1" // hack TODO fix this up
            }
        })
        .as_object()
        .cloned()
        .expect("json object"),
        limit: args.limit,
        page_token: args.page_token,
    };

    let page = if args.text_only {
        searcher.search_text(&args.query, &params).await?
    } else {
        let embedder = search::gcloud::GeminiEmbedder::from_config(&state.config)?;
        let query_embedding = embedder.embed_query(&args.query).await?;
        tracing::info!(
            "Generated query embedding with dims={}",
            query_embedding.len()
        );

        if args.vector_only {
            searcher.search_embedding(&query_embedding, &params).await?
        } else {
            searcher
                .search_hybrid(&args.query, &query_embedding, &params)
                .await?
        }
    };

    println!("Found {} hit(s) for query: {}", page.hits.len(), args.query);
    for hit in page.hits {
        println!("object_id={} score={}", hit.object_id, hit.score,);
    }

    if let Some(next_page_token) = page.next_page_token {
        println!("next_page_token={}", next_page_token);
    }

    Ok(())
}
