use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter};

/// Shared v2 taxonomy for persisted knowledge entities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Display, EnumIter, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum KnowledgeEntityType {
    RealPerson,
    ReligiousFigure,
    Place,
    Event,
    Book,
    Publication,
    Movie,
    TelevisionShow,
    ArtWork,
    FictionalCharacter,
    Music,
    Meme,
    Software,
    Product,
    Financial,
    Brand,
    Organization,
    OnlineCommunity,
    SocialMediaAccount,
    Unknown,
}
