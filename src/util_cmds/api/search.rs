use super::*;
use argh::FromArgs;

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

    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

pub async fn run(state: ApiCmdState, args: SearchArgs) -> anyhow::Result<()> {
    let captures = state
        .rest_client
        .search(&args.query, Some(args.limit as u64))
        .await?;
    if args.raw_json {
        return print_json(&captures);
    }
    println!(
        "Found {} capture(s) for query: {}",
        captures.len(),
        args.query
    );
    for capture in captures {
        println!(
            "capture_id={} created_at={}",
            capture.id, capture.created_at
        );
    }
    Ok(())
}
