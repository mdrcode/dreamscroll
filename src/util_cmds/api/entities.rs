use super::*;
use argh::FromArgs;

#[derive(FromArgs)]
#[argh(subcommand, name = "unknown-entities")]
#[argh(description = "List entities whose type is unknown")]
pub struct UnknownEntitiesArgs {}

pub async fn run(state: ApiCmdState, _args: UnknownEntitiesArgs) -> anyhow::Result<()> {
    let entities = state.rest_client.get_entities_by_type("unknown").await?;
    eprintln!("Retrieved {} unknown entities", entities.len());
    println!("{}", serde_json::to_string_pretty(&entities)?);
    Ok(())
}
