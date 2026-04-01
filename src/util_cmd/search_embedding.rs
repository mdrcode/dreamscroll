use anyhow::Context;
use argh::FromArgs;

use crate::search::{Embedding, QueryParams, Searcher, gcloud::VertexVectorSearcher};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "search_embedding")]
#[argh(description = "Search captures by embedding vector JSON file")]
pub struct SearchEmbeddingArgs {
    #[argh(positional)]
    #[argh(description = "path to vector JSON file")]
    vector_file: String,

    #[argh(option, default = "20")]
    #[argh(description = "maximum number of results to return")]
    limit: u32,

    #[argh(option)]
    #[argh(description = "optional page token for pagination")]
    page_token: Option<String>,
}

pub async fn run(state: CmdState, args: SearchEmbeddingArgs) -> anyhow::Result<()> {
    let searcher = VertexVectorSearcher::from_config(&state.config).await?;

    let params = QueryParams {
        user_id: 1, // hack TODO fix this up
        limit: args.limit,
        page_token: args.page_token,
    };

    let query_embedding = read_query_vector_from_file(&args.vector_file)?;
    tracing::info!(
        path = args.vector_file,
        dims = query_embedding.len(),
        "Loaded query embedding from file"
    );

    let page = searcher.search_embedding(&query_embedding, &params).await?;

    println!(
        "Found {} hit(s) for user_id: {} vector_file: {}",
        page.hits.len(),
        params.user_id,
        args.vector_file
    );
    for hit in page.hits {
        println!("object_id={} score={}", hit.object_id, hit.score,);
    }

    if let Some(next_page_token) = page.next_page_token {
        println!("next_page_token={}", next_page_token);
    }

    Ok(())
}

fn read_query_vector_from_file(
    file_path: &str,
) -> anyhow::Result<Embedding<f32, crate::search::Unit>> {
    let raw = std::fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read vector file: {}", file_path))?;

    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("Invalid JSON in vector file: {}", file_path))?;

    let arr = value
        .pointer("/dense/values")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.pointer("/values").and_then(serde_json::Value::as_array))
        .or_else(|| value.as_array())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Vector file must contain either {{\"dense\":{{\"values\":[...]}}}}, {{\"values\":[...]}}, or a raw array"
            )
        })?;

    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let Some(v) = item.as_f64() else {
            anyhow::bail!("Vector value at index {} is not numeric", idx);
        };
        out.push(v as f32);
    }

    if out.is_empty() {
        anyhow::bail!("Vector file contains an empty embedding");
    }

    Embedding::from_vec_normalizing(out)
}
