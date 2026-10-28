use super::*;
use argh::FromArgs;

#[derive(FromArgs)]
#[argh(subcommand, name = "captures")]
#[argh(description = "Get captures by ID(s)")]
pub struct CapturesArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) or ranges (e.g. 205-209) of captures to fetch")]
    ids: Vec<String>,

    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

pub async fn run(state: ApiCmdState, args: CapturesArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow::anyhow!("At least one capture ID must be provided."));
    }

    let capture_ids = parse_capture_ids(&args.ids)?;
    let captures = state.rest_client.get_captures(Some(&capture_ids)).await?;

    if args.raw_json {
        return print_json(&captures);
    }

    println!(
        "Found {} of {} capture(s)",
        captures.len(),
        capture_ids.len()
    );
    for capture in &captures {
        println!(
            "capture_id={} user_id={} created_at={}",
            capture.id, capture.user_id, capture.created_at
        );
        for media in &capture.medias {
            println!(
                "  media_id={} mime_type={} url={}",
                media.id,
                media.mime_type.as_deref().unwrap_or("-"),
                media.url
            );
        }
    }

    let missing = capture_ids
        .iter()
        .filter(|id| !captures.iter().any(|capture| capture.id == **id))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        println!(
            "Not found: {}",
            missing
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}
