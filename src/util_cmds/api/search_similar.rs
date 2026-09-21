use argh::FromArgs;

use super::*;
#[derive(FromArgs)]
#[argh(subcommand, name = "search_similar")]
#[argh(description = "Search captures similar to a given capture ID")]
pub struct SearchSimilarArgs {
    #[argh(positional)]
    #[argh(description = "query capture ID")]
    capture_id: i32,

    #[argh(option, default = "20")]
    #[argh(description = "maximum number of results to return")]
    limit: u64,
}

pub async fn run(state: ApiCmdState, args: SearchSimilarArgs) -> anyhow::Result<()> {
    let capture_infos = state
        .client
        .search_similar(args.capture_id, Some(args.limit))
        .await?;

    println!(
        "Found {} similar capture(s) for capture_id={}",
        capture_infos.len(),
        args.capture_id
    );
    for capture in capture_infos {
        println!(
            "capture_id={} created_at={}",
            capture.id, capture.created_at
        );
    }

    Ok(())
}
