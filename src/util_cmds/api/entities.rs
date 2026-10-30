use anyhow::Context;
use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "unknown-entities")]
#[argh(description = "List entities whose type is unknown")]
pub struct UnknownEntitiesArgs {
    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the table")]
    raw_json: bool,
}

pub async fn run(state: ApiCmdState, args: UnknownEntitiesArgs) -> anyhow::Result<()> {
    let entities = state.rest_client.get_entities_by_type("unknown").await?;
    eprintln!("Retrieved {} unknown entities", entities.len());
    if args.raw_json {
        return print_json(&entities);
    }
    if entities.is_empty() {
        println!("(none)");
        return Ok(());
    }
    let name_width = entities
        .iter()
        .map(|entity| entity.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("  {:<8} {:<name_width$} {:>10}", "id", "name", "capture_id");
    for entity in &entities {
        println!(
            "  {:<8} {:<name_width$} {:>10}",
            entity.id, entity.name, entity.capture_id
        );
    }
    Ok(())
}

#[derive(FromArgs)]
#[argh(subcommand, name = "entity")]
#[argh(description = "Fetch one knowledge entity by ID (must be your own)")]
pub struct GetEntityArgs {
    #[argh(positional, description = "knowledge entity ID")]
    id: i32,
}

pub async fn get(state: ApiCmdState, args: GetEntityArgs) -> anyhow::Result<()> {
    let entity = state.rest_client.get_entity(args.id).await?;
    print_json(&entity)
}

#[derive(FromArgs)]
#[argh(subcommand, name = "export-entities")]
#[argh(description = "Export knowledge entities as JSONL, one ExportRow per line (admin only)")]
pub struct ExportEntitiesArgs {
    #[argh(
        option,
        long = "out",
        description = "output file (JSONL, one user_id + row object per line)"
    )]
    out: String,

    #[argh(
        option,
        long = "page-size",
        default = "1000",
        description = "rows per request (max 5000)"
    )]
    page_size: u64,

    #[argh(
        option,
        long = "user-id",
        description = "export one user only; omit for every user in one stream"
    )]
    user_id: Option<i32>,

    #[argh(option, long = "max", description = "stop after this many rows")]
    max: Option<usize>,

    #[argh(switch, description = "silence per-page progress on stderr")]
    quiet: bool,
}

pub async fn export(state: ApiCmdState, args: ExportEntitiesArgs) -> anyhow::Result<()> {
    use std::io::Write;

    let file = std::fs::File::create(&args.out)
        .with_context(|| format!("failed to create {}", args.out))?;
    let mut writer = std::io::BufWriter::new(file);

    let mut after_id = 0;
    let mut total = 0usize;

    loop {
        // Trim the last request to the remaining `--max` budget so the file ends
        // at exactly that count rather than at a page boundary.
        let remaining = args
            .max
            .map_or(args.page_size, |max| max.saturating_sub(total) as u64);
        let limit = args.page_size.min(remaining);
        if limit == 0 {
            break;
        }

        let (rows, next_after_id) = state
            .rest_client
            .export_entities(args.user_id, after_id, limit)
            .await?;

        for row in &rows {
            serde_json::to_writer(&mut writer, row)?;
            writer.write_all(b"\n")?;
        }

        total += rows.len();
        if !args.quiet {
            eprintln!(
                "exported {total} rows (through entity id {})",
                rows.last().map(|row| row.row.id).unwrap_or(after_id)
            );
        }

        match next_after_id {
            Some(next) => after_id = next,
            None => break,
        }
    }

    writer.flush()?;
    eprintln!("wrote {total} rows to {}", args.out);
    Ok(())
}
