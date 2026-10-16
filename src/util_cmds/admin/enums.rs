use argh::FromArgs;

use crate::illumination::{KnowledgeEntityType, v2::Platform};

#[derive(FromArgs)]
#[argh(subcommand, name = "enums")]
#[argh(description = "List knowledge entity types and platform-link platforms")]
pub struct EnumsArgs {}

pub async fn run(_args: EnumsArgs) -> anyhow::Result<()> {
    let knowledge_entity_types: Vec<String> = {
        use strum::IntoEnumIterator;
        KnowledgeEntityType::iter()
            .map(|entity_type| entity_type.as_ref().to_string())
            .collect()
    };

    println!("Knowledge Entity Types:");
    println!("{}", knowledge_entity_types.join(", "));

    let platforms: Vec<String> = {
        use strum::IntoEnumIterator;
        Platform::iter()
            .map(|platform| platform.as_ref().to_string())
            .collect()
    };

    println!("\nPlatform-Link Platforms:");
    println!("{}", platforms.join(", "));

    Ok(())
}
