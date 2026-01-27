use argh::FromArgs;

use crate::{facility, illumination::EntityType};

#[derive(FromArgs)]
#[argh(subcommand, name = "knodes")]
#[argh(description = "List all unique knode entity types")]
pub struct KNodesArgs {}

pub async fn run(_config: facility::Config, _args: KNodesArgs) -> anyhow::Result<()> {
    let entity_type_enum: Vec<String> = {
        use strum::IntoEnumIterator;
        EntityType::iter().map(|e| e.as_ref().to_string()).collect()
    };

    // Print as comma-delimited list
    println!("{}", entity_type_enum.join(", "));

    Ok(())
}
