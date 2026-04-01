use argh::FromArgs;

use crate::search::{self, *};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search_text")]
#[argh(description = "Search captures by text query using Vertext AI Search")]
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

    #[argh(switch)]
    #[argh(description = "run text-only search")]
    text_only: bool,

    #[argh(switch)]
    #[argh(description = "run vector-only search")]
    vector_only: bool,
}

pub async fn run(state: CmdState, args: SearchQueryArgs) -> anyhow::Result<()> {
    let searcher = search::gcloud::VertexVectorSearcher::from_config(&state.config).await?;

    if args.text_only && args.vector_only {
        anyhow::bail!("Choose at most one mode: --text-only or --vector-only");
    }

    let params = QueryParams {
        user_id: 1, // hack TODO fix this up
        limit: args.limit,
        page_token: args.page_token,
    };

    let page = if args.text_only {
        searcher.search_text(&args.query, &params).await?
    } else {
        let embedder =
            search::gcloud::GeminiEmbedder::from_config(&state.config, state.stg.clone())?;
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

    println!(
        "Found {} hit(s) for user_id: {} query: {}",
        page.hits.len(),
        params.user_id,
        args.query
    );
    for hit in page.hits {
        println!("object_id={} score={}", hit.object_id, hit.score,);
    }

    if let Some(next_page_token) = page.next_page_token {
        println!("next_page_token={}", next_page_token);
    }

    Ok(())
}
