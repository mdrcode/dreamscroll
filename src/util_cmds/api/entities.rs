use super::*;
use argh::FromArgs;

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
