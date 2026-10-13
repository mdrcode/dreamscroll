use argh::FromArgs;

use crate::illumination::{KnowledgeEntityType, v1::SocialMediaPlatform};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "enums")]
#[argh(description = "List unique illumination entity types and social media platforms")]
pub struct EnumsArgs {}

pub async fn run(_state: AdminCmdState, _args: EnumsArgs) -> anyhow::Result<()> {
    let knowledge_entity_types: Vec<String> = {
        use strum::IntoEnumIterator;
        KnowledgeEntityType::iter()
            .map(|entity_type| entity_type.as_ref().to_string())
            .collect()
    };

    println!("Knowledge Entity Types:");
    println!("{}", knowledge_entity_types.join(", "));

    let social_media_platforms: Vec<String> = {
        use strum::IntoEnumIterator;
        SocialMediaPlatform::iter()
            .map(|e| e.as_ref().to_string())
            .collect()
    };

    println!("\nSocial Media Platforms:");
    println!("{}", social_media_platforms.join(", "));

    Ok(())
}
